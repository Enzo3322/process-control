use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::Command,
};

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    models::{DockerContainer, PartialProcess, ProcessInfo},
    state::AppState,
};

pub(crate) fn list_monitored_processes<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<ProcessInfo>, String> {
    let ports = app.state::<AppState>().configured_ports()?;
    let mut entries = read_listening_processes(&ports)?;
    entries.sort_by(|left, right| left.port.cmp(&right.port).then(left.pid.cmp(&right.pid)));
    Ok(entries)
}

pub(crate) fn terminate_target(port: u16, pid: u32) -> Result<(), String> {
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

pub(crate) fn open_config_in_vscode(path: &Path) -> Result<(), String> {
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
        .filter_map(|((port, _), item)| {
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
