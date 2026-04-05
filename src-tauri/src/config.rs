use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Manager, Runtime};

use crate::{models::{ConfigSnapshot, PortsConfig}, state::AppState};

const CONFIG_FILE_NAME: &str = "ports.json";

pub(crate) fn config_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(config_dir(app)?.join(CONFIG_FILE_NAME))
}

pub(crate) fn load_config_snapshot<R: Runtime>(app: &AppHandle<R>) -> Result<ConfigSnapshot, String> {
    let path = config_path(app)?;
    let config = load_ports_config_from_path(&path)?;
    let ports = parse_configured_ports(&config)?;

    Ok(ConfigSnapshot { path, ports })
}

pub(crate) fn reload_cached_config<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let snapshot = load_config_snapshot(app)?;
    app.state::<AppState>().replace_config(snapshot)
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
    serde_json::from_str::<PortsConfig>(&content).map_err(|error| {
        format!(
            "Configuracao de portas invalida em {}: {error}. Use o formato {{\"ports\":\"3000-3002,5432,6379\"}}.",
            path.display()
        )
    })
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
