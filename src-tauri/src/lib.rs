use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
    thread,
    time::Duration,
};

use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
use serde::{Deserialize, Serialize};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

const TRAY_ID: &str = "process-control-tray";
const CONFIG_FILE_NAME: &str = "ports.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PortsConfig {
    ports: String,
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyPortRange {
    start: u16,
    end: u16,
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyPortsConfig {
    exact_ports: Vec<u16>,
    ranges: Vec<LegacyPortRange>,
}

impl Default for PortsConfig {
    fn default() -> Self {
        Self {
            ports: "80,3000-3001,3333,4000,4200,4321,5000,5173,5432,6379,8000-8100,8888-8890"
                .to_string(),
        }
    }
}

#[derive(Debug, Clone)]
struct ProcessInfo {
    port: u16,
    pid: u32,
    name: String,
    address: String,
}

#[derive(Debug, Clone)]
struct DockerContainer {
    id: String,
    name: String,
}

#[derive(Debug, Default)]
struct PartialProcess {
    pid: Option<u32>,
    name: Option<String>,
    address: Option<String>,
}

#[derive(Debug, Clone)]
struct ConfigSnapshot {
    path: PathBuf,
    ports: BTreeSet<u16>,
}

struct AppState {
    config: Mutex<ConfigSnapshot>,
}

impl AppState {
    fn new(config: ConfigSnapshot) -> Self {
        Self {
            config: Mutex::new(config),
        }
    }

    fn config_path(&self) -> Result<PathBuf, String> {
        self.config
            .lock()
            .map_err(|_| "Falha ao acessar o cache de configuracao.".to_string())
            .map(|snapshot| snapshot.path.clone())
    }

    fn configured_ports(&self) -> Result<BTreeSet<u16>, String> {
        self.config
            .lock()
            .map_err(|_| "Falha ao acessar o cache de configuracao.".to_string())
            .map(|snapshot| snapshot.ports.clone())
    }

    fn replace_config(&self, config: ConfigSnapshot) -> Result<(), String> {
        let mut snapshot = self
            .config
            .lock()
            .map_err(|_| "Falha ao atualizar o cache de configuracao.".to_string())?;
        *snapshot = config;
        Ok(())
    }
}

fn config_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("Falha ao localizar a pasta de configuracao: {error}"))?;

    fs::create_dir_all(&config_dir)
        .map_err(|error| format!("Falha ao preparar a pasta de configuracao: {error}"))?;

    Ok(config_dir)
}

fn config_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(config_dir(app)?.join(CONFIG_FILE_NAME))
}

fn load_ports_config_from_path(path: &Path) -> Result<PortsConfig, String> {
    if !path.exists() {
        let default_config = PortsConfig::default();
        let content = serde_json::to_string_pretty(&default_config)
            .map_err(|error| format!("Falha ao serializar a configuracao padrao: {error}"))?;
        fs::write(path, content)
            .map_err(|error| format!("Falha ao criar a configuracao inicial: {error}"))?;
        return Ok(default_config);
    }

    let content = fs::read_to_string(path)
        .map_err(|error| format!("Falha ao ler a configuracao de portas: {error}"))?;

    match serde_json::from_str::<PortsConfig>(&content) {
        Ok(config) => Ok(config),
        Err(primary_error) => {
            let legacy: LegacyPortsConfig = serde_json::from_str(&content).map_err(|_| {
                format!(
                    "Configuracao de portas invalida em {}: {primary_error}",
                    path.display()
                )
            })?;

            let migrated = migrate_legacy_config(legacy);
            let migrated_content = serde_json::to_string_pretty(&migrated).map_err(|error| {
                format!("Falha ao serializar a configuracao migrada: {error}")
            })?;
            fs::write(path, migrated_content)
                .map_err(|error| format!("Falha ao salvar a configuracao migrada: {error}"))?;

            Ok(migrated)
        }
    }
}

fn migrate_legacy_config(legacy: LegacyPortsConfig) -> PortsConfig {
    let mut entries = legacy
        .exact_ports
        .into_iter()
        .map(|port| port.to_string())
        .collect::<Vec<_>>();

    entries.extend(
        legacy
            .ranges
            .into_iter()
            .map(|range| format!("{}-{}", range.start, range.end)),
    );

    PortsConfig {
        ports: entries.join(","),
    }
}

fn parse_configured_ports(config: &PortsConfig) -> Result<BTreeSet<u16>, String> {
    let mut ports = BTreeSet::new();

    for segment in config.ports.split(',') {
        let segment = segment.trim();

        if segment.is_empty() {
            continue;
        }

        if let Some((start, end)) = segment.split_once('-') {
            let start = start
                .trim()
                .parse::<u16>()
                .map_err(|_| format!("Porta inicial invalida no trecho '{segment}'."))?;
            let end = end
                .trim()
                .parse::<u16>()
                .map_err(|_| format!("Porta final invalida no trecho '{segment}'."))?;

            if start > end {
                return Err(format!(
                    "Range invalido no trecho '{segment}': inicio {start} maior que fim {end}."
                ));
            }

            for port in start..=end {
                ports.insert(port);
            }
        } else {
            let port = segment
                .parse::<u16>()
                .map_err(|_| format!("Porta invalida no trecho '{segment}'."))?;
            ports.insert(port);
        }
    }

    if ports.is_empty() {
        return Err("Nenhuma porta foi configurada em ports.json.".to_string());
    }

    Ok(ports)
}

fn load_config_snapshot<R: Runtime>(app: &AppHandle<R>) -> Result<ConfigSnapshot, String> {
    let path = config_path(app)?;
    let config = load_ports_config_from_path(&path)?;
    let ports = parse_configured_ports(&config)?;

    Ok(ConfigSnapshot { path, ports })
}

fn reload_cached_config<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let snapshot = load_config_snapshot(app)?;
    app.state::<AppState>().replace_config(snapshot)
}

fn list_common_port_processes<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<ProcessInfo>, String> {
    let ports = app.state::<AppState>().configured_ports()?;
    let mut entries = read_listening_processes(&ports)?;

    entries.sort_by(|left, right| left.port.cmp(&right.port).then(left.pid.cmp(&right.pid)));

    Ok(entries)
}

fn terminate_process(pid: u32) -> Result<(), String> {
    let forced = Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .status()
        .map_err(|error| format!("Falha ao forcar o encerramento do PID {pid}: {error}"))?;

    if forced.success() {
        Ok(())
    } else {
        Err(format!("O sistema recusou encerrar o PID {pid}."))
    }
}

fn terminate_target(port: u16, pid: u32) -> Result<(), String> {
    if let Some(container) = docker_container_for_port(port)? {
        return terminate_docker_container(&container);
    }

    if is_probably_docker_process(pid)? {
        return Err(format!(
            "A porta {port} parece estar publicada por Docker, mas nenhum container foi identificado com seguranca."
        ));
    }

    terminate_process(pid)
}

fn docker_container_for_port(port: u16) -> Result<Option<DockerContainer>, String> {
    let output = match Command::new("docker")
        .args([
            "ps",
            "--filter",
            &format!("publish={port}"),
            "--format",
            "{{.ID}}\t{{.Names}}",
        ])
        .output()
    {
        Ok(output) => output,
        Err(_) => return Ok(None),
    };

    if !output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().find(|line| !line.trim().is_empty());
    let Some(line) = line else {
        return Ok(None);
    };

    let mut parts = line.split('\t');
    let id = parts.next().unwrap_or_default().trim();
    let name = parts.next().unwrap_or_default().trim();

    if id.is_empty() {
        return Ok(None);
    }

    Ok(Some(DockerContainer {
        id: id.to_string(),
        name: if name.is_empty() {
            "container".to_string()
        } else {
            name.to_string()
        },
    }))
}

fn terminate_docker_container(container: &DockerContainer) -> Result<(), String> {
    let status = Command::new("docker")
        .args(["kill", &container.id])
        .status()
        .map_err(|error| {
            format!(
                "Falha ao encerrar o container Docker {}: {error}",
                container.name
            )
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "O Docker recusou encerrar o container {}.",
            container.name
        ))
    }
}

fn is_probably_docker_process(pid: u32) -> Result<bool, String> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .map_err(|error| format!("Falha ao inspecionar o PID {pid}: {error}"))?;

    if !output.status.success() {
        return Ok(false);
    }

    let command = String::from_utf8_lossy(&output.stdout).to_lowercase();
    Ok(command.contains("docker"))
}

fn open_config_in_vscode(path: &Path) -> Result<(), String> {
    let status = Command::new("open")
        .args(["-a", "Visual Studio Code"])
        .arg(path)
        .status()
        .map_err(|error| format!("Falha ao abrir o VS Code: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err("Nao foi possivel abrir o arquivo de configuracao no VS Code.".to_string())
    }
}

fn read_listening_processes(configured_ports: &BTreeSet<u16>) -> Result<Vec<ProcessInfo>, String> {
    let output = Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpcn"])
        .output()
        .map_err(|error| format!("Falha ao consultar processos TCP em escuta: {error}"))?;

    if !output.status.success() && !output.stdout.is_empty() {
        return Err("Nao foi possivel ler os processos TCP em escuta.".to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut by_socket: BTreeMap<(u16, u32), PartialProcess> = BTreeMap::new();
    let mut current_pid: Option<u32> = None;
    let mut current_name: Option<String> = None;

    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }

        let mut chars = line.chars();
        let field = chars.next().unwrap_or_default();
        let value = chars.as_str().trim();

        match field {
            'p' => {
                let pid = value
                    .parse::<u32>()
                    .map_err(|_| "PID invalido retornado pelo lsof.".to_string())?;
                current_pid = Some(pid);
                current_name = None;
            }
            'c' => {
                current_name = Some(value.to_string());
            }
            'n' => {
                if let Some((port, pid, address)) = parse_listener_entry(value, current_pid) {
                    if !configured_ports.contains(&port) {
                        continue;
                    }

                    let entry = by_socket.entry((port, pid)).or_default();
                    entry.pid = Some(pid);
                    entry.address = Some(address);
                    if let Some(name) = &current_name {
                        entry.name = Some(name.clone());
                    }
                }
            }
            _ => {}
        }
    }

    Ok(by_socket
        .into_iter()
        .filter_map(|((port, _pid), item)| {
            Some(ProcessInfo {
                port,
                pid: item.pid?,
                name: item.name.unwrap_or_else(|| "process".to_string()),
                address: item.address.unwrap_or_else(|| format!("*:{port}")),
            })
        })
        .collect())
}

fn parse_listener_entry(value: &str, pid: Option<u32>) -> Option<(u16, u32, String)> {
    let pid = pid?;
    let normalized = value.trim();
    let address_part = normalized
        .split(" (LISTEN)")
        .next()
        .unwrap_or(normalized)
        .trim();

    let port_text = address_part.rsplit(':').next()?.trim();
    let port = port_text.parse::<u16>().ok()?;

    Some((port, pid, address_part.to_string()))
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

fn menu_bar_title(active_ports: usize) -> String {
    if active_ports == 0 {
        "Ports".to_string()
    } else {
        format!("Ports {active_ports}")
    }
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

fn build_tray_menu<R: Runtime>(
    app: &AppHandle<R>,
    processes: &[ProcessInfo],
    error_message: Option<&str>,
) -> tauri::Result<Menu<R>> {
    let menu = Menu::new(app)?;
    let mut grouped = BTreeMap::<u16, Vec<&ProcessInfo>>::new();

    for process in processes {
        grouped.entry(process.port).or_default().push(process);
    }

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

fn refresh_tray<R: Runtime>(app: &AppHandle<R>, error_message: Option<&str>) -> Result<(), String> {
    let processes = list_common_port_processes(app)?;
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

fn refresh_tray_with_message<R: Runtime>(app: &AppHandle<R>, message: &str) {
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

#[cfg(target_os = "macos")]
fn open_tray_menu<R: Runtime>(tray: &tauri::tray::TrayIcon<R>) -> Result<(), String> {
    tray.with_inner_tray_icon(|inner| {
        let Some(status_item) = inner.ns_status_item() else {
            return Err("Nao foi possivel acessar o item da menubar.".to_string());
        };

        unsafe {
            let mtm = MainThreadMarker::new().ok_or_else(|| {
                "Nao foi possivel acessar a main thread do macOS.".to_string()
            })?;
            let button = status_item
                .button(mtm)
                .ok_or_else(|| "Nao foi possivel acessar o botao da menubar.".to_string())?;
            button.performClick(None);
        }

        Ok(())
    })
    .map_err(|error| format!("Falha ao abrir o menu da menubar: {error}"))?
}

fn create_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
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

            if let Some((port, pid)) = id
                .strip_prefix("kill:")
                .and_then(|value| {
                    let (port, pid) = value.split_once(':')?;
                    Some((port.parse::<u16>().ok()?, pid.parse::<u32>().ok()?))
                })
            {
                match terminate_target(port, pid) {
                    Ok(()) => schedule_refresh(app.clone(), None),
                    Err(error) => schedule_refresh(app.clone(), Some(error)),
                }
            }
        })
        .build(app)?;

    Ok(())
}

fn start_process_refresh_loop<R: Runtime>(app: AppHandle<R>) {
    thread::spawn(move || loop {
        if let Err(error) = refresh_tray(&app, None) {
            refresh_tray_with_message(&app, &error);
        }

        thread::sleep(Duration::from_secs(10));
    });
}

fn start_config_watcher<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
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
                refresh_tray_with_message(
                    &app,
                    &format!("Falha ao observar ports.json: {error}"),
                );
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

            let is_target_file = event.paths.iter().any(|path| path == &watched_file);

            if !is_target_file {
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let snapshot = load_config_snapshot(&app.handle())?;
            app.manage(AppState::new(snapshot));

            create_tray(&app.handle())?;
            start_config_watcher(app.handle().clone())?;
            start_process_refresh_loop(app.handle().clone());

            if let Err(error) = refresh_tray(&app.handle(), None) {
                refresh_tray_with_message(&app.handle(), &error);
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
