use std::{collections::{BTreeMap, BTreeSet}, thread, time::Duration};

use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

use crate::{
    config::{reload_cached_config},
    models::ProcessInfo,
    runtime_ops::{list_monitored_processes, open_config_in_vscode, terminate_target},
    state::AppState,
};

const TRAY_ID: &str = "process-control-tray";

pub(crate) fn create_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let initial_menu = build_tray_menu(app, &[], Some("Carregando portas..."))?;

    TrayIconBuilder::with_id(TRAY_ID)
        .menu(&initial_menu)
        .title("Ports")
        .tooltip("Process Control")
        .show_menu_on_left_click(false)
        .icon_as_template(true)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Down,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Err(error) = refresh_tray(app, None) {
                    refresh_tray_with_message(app, &error);
                    return;
                }

                #[cfg(target_os = "macos")]
                if let Err(error) = open_tray_menu(tray) {
                    refresh_tray_with_message(app, &error);
                }
            }
        })
        .on_menu_event(|app, event| {
            let id = event.id.as_ref();

            if id == "edit-config" {
                match app
                    .state::<AppState>()
                    .config_path()
                    .and_then(|path| open_config_in_vscode(&path))
                {
                    Ok(()) => {}
                    Err(error) => schedule_refresh(app.clone(), Some(error)),
                }
                return;
            }

            if id == "quit" {
                app.exit(0);
                return;
            }

            if let Some((port, pid)) = id.strip_prefix("kill:").and_then(parse_kill_id) {
                match terminate_target(port, pid) {
                    Ok(()) => schedule_refresh(app.clone(), None),
                    Err(error) => schedule_refresh(app.clone(), Some(error)),
                }
            }
        })
        .build(app)?;

    Ok(())
}

pub(crate) fn start_process_refresh_loop<R: Runtime>(app: AppHandle<R>) {
    thread::spawn(move || loop {
        if let Err(error) = refresh_tray(&app, None) {
            refresh_tray_with_message(&app, &error);
        }

        thread::sleep(Duration::from_secs(10));
    });
}

pub(crate) fn start_config_watcher<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let config_path = app.state::<AppState>().config_path()?;
    let watch_dir = config_path
        .parent()
        .ok_or_else(|| "Nao foi possivel determinar a pasta da configuracao.".to_string())?
        .to_path_buf();

    thread::spawn(move || {
        let watched_file = config_path;
        let (tx, rx) = std::sync::mpsc::channel();

        let mut watcher = match RecommendedWatcher::new(
            move |result| {
                let _ = tx.send(result);
            },
            NotifyConfig::default(),
        ) {
            Ok(watcher) => watcher,
            Err(error) => {
                refresh_tray_with_message(&app, &format!("Falha ao observar ports.json: {error}"));
                return;
            }
        };

        if let Err(error) = watcher.watch(&watch_dir, RecursiveMode::NonRecursive) {
            refresh_tray_with_message(
                &app,
                &format!("Falha ao observar a pasta de configuracao: {error}"),
            );
            return;
        }

        while let Ok(result) = rx.recv() {
            let event = match result {
                Ok(event) => event,
                Err(error) => {
                    refresh_tray_with_message(
                        &app,
                        &format!("Erro no monitoramento do ports.json: {error}"),
                    );
                    continue;
                }
            };

            if !event.paths.iter().any(|path| path == &watched_file) {
                continue;
            }

            match reload_cached_config(&app).and_then(|_| refresh_tray(&app, None)) {
                Ok(()) => {}
                Err(error) => refresh_tray_with_message(&app, &error),
            }
        }
    });

    Ok(())
}

pub(crate) fn refresh_tray<R: Runtime>(
    app: &AppHandle<R>,
    error_message: Option<&str>,
) -> Result<(), String> {
    let processes = list_monitored_processes(app)?;
    let active_ports = processes
        .iter()
        .map(|process| process.port)
        .collect::<BTreeSet<_>>()
        .len();
    let menu =
        build_tray_menu(app, &processes, error_message).map_err(|error| error.to_string())?;
    let tray = app
        .tray_by_id(TRAY_ID)
        .ok_or_else(|| "Tray icon nao encontrada.".to_string())?;

    tray.set_menu(Some(menu))
        .map_err(|error| format!("Falha ao atualizar o menu: {error}"))?;
    let _ = tray.set_title(Some(menu_bar_title(active_ports)));

    Ok(())
}

pub(crate) fn refresh_tray_with_message<R: Runtime>(app: &AppHandle<R>, message: &str) {
    let menu = build_tray_menu(app, &[], Some(message));

    if let Ok(menu) = menu {
        if let Some(tray) = app.tray_by_id(TRAY_ID) {
            let _ = tray.set_menu(Some(menu));
            let _ = tray.set_title(Some("Ports"));
        }
    }
}

fn schedule_refresh<R: Runtime>(app: AppHandle<R>, message: Option<String>) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(250));

        match message {
            Some(message) => refresh_tray_with_message(&app, &message),
            None => {
                if let Err(error) = refresh_tray(&app, None) {
                    refresh_tray_with_message(&app, &error);
                }
            }
        }
    });
}

fn build_tray_menu<R: Runtime>(
    app: &AppHandle<R>,
    processes: &[ProcessInfo],
    error_message: Option<&str>,
) -> tauri::Result<Menu<R>> {
    let menu = Menu::new(app)?;
    let grouped = group_processes_by_port(processes);

    let summary = if let Some(message) = error_message {
        MenuItem::with_id(app, "summary:error", message, false, None::<&str>)?
    } else {
        MenuItem::with_id(
            app,
            "summary:ok",
            format!("{} porta(s) ativa(s) · {} processo(s)", grouped.len(), processes.len()),
            false,
            None::<&str>,
        )?
    };

    menu.append(&summary)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    if grouped.is_empty() {
        let empty = MenuItem::with_id(
            app,
            "ports:empty",
            "Nenhuma porta configurada esta em uso agora",
            false,
            None::<&str>,
        )?;
        menu.append(&empty)?;
    } else {
        for (port, items) in grouped {
            let submenu = build_process_submenu(app, port, &items)?;
            menu.append(&submenu)?;
        }
    }

    menu.append(&PredefinedMenuItem::separator(app)?)?;

    let edit = MenuItem::with_id(app, "edit-config", "Edit", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
    menu.append(&edit)?;
    menu.append(&quit)?;

    Ok(menu)
}

fn build_process_submenu<R: Runtime>(
    app: &AppHandle<R>,
    port: u16,
    processes: &[&ProcessInfo],
) -> tauri::Result<Submenu<R>> {
    let title = format!(
        ":{} · {} · {} ativo{}",
        port,
        port_label(port),
        processes.len(),
        if processes.len() > 1 { "s" } else { "" }
    );

    let submenu = Submenu::new(app, title, true)?;

    for process in processes {
        let info = MenuItem::with_id(
            app,
            format!("info:{}:{}", process.port, process.pid),
            format!("{} · PID {} · TCP", process.name, process.pid),
            false,
            None::<&str>,
        )?;
        let address = MenuItem::with_id(
            app,
            format!("address:{}:{}", process.port, process.pid),
            process.address.clone(),
            false,
            None::<&str>,
        )?;
        let kill = MenuItem::with_id(
            app,
            format!("kill:{}:{}", process.port, process.pid),
            "Encerrar processo",
            true,
            None::<&str>,
        )?;

        submenu.append(&info)?;
        submenu.append(&address)?;
        submenu.append(&kill)?;
        submenu.append(&PredefinedMenuItem::separator(app)?)?;
    }

    Ok(submenu)
}

fn group_processes_by_port(processes: &[ProcessInfo]) -> BTreeMap<u16, Vec<&ProcessInfo>> {
    let mut grouped = BTreeMap::new();

    for process in processes {
        grouped.entry(process.port).or_insert_with(Vec::new).push(process);
    }

    grouped
}

fn parse_kill_id(value: &str) -> Option<(u16, u32)> {
    let (port, pid) = value.split_once(':')?;
    Some((port.parse::<u16>().ok()?, pid.parse::<u32>().ok()?))
}

fn menu_bar_title(active_ports: usize) -> String {
    if active_ports == 0 {
        "Ports".to_string()
    } else {
        format!("Ports {active_ports}")
    }
}

fn port_label(port: u16) -> &'static str {
    match port {
        80 => "HTTP",
        3000 => "Frontend",
        3001 => "Alt Frontend",
        3333 => "API",
        4000 => "Backend",
        4200 => "Angular",
        4321 => "Wasp",
        5000 => "Flask/Rails",
        5173 => "Vite",
        5432 => "Postgres",
        6379 => "Redis",
        8000 => "Dev Server",
        8080 => "Proxy",
        8081 => "Alt Proxy",
        8888 => "Jupyter",
        _ => "Porta comum",
    }
}

#[cfg(target_os = "macos")]
fn open_tray_menu<R: Runtime>(tray: &TrayIcon<R>) -> Result<(), String> {
    tray.with_inner_tray_icon(|inner| {
        let Some(status_item) = inner.ns_status_item() else {
            return Err("Nao foi possivel acessar o item da menubar.".to_string());
        };

        unsafe {
            let mtm = MainThreadMarker::new()
                .ok_or_else(|| "Nao foi possivel acessar a main thread do macOS.".to_string())?;
            let button = status_item
                .button(mtm)
                .ok_or_else(|| "Nao foi possivel acessar o botao da menubar.".to_string())?;
            button.performClick(None);
        }

        Ok(())
    })
    .map_err(|error| format!("Falha ao abrir o menu da menubar: {error}"))?
}
