use std::{collections::BTreeSet, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PortsConfig {
    pub(crate) ports: String,
}

impl Default for PortsConfig {
    fn default() -> Self {
        Self {
            ports: "80,3000-3001,3333,4000,4200,4321,5000,5173,5432,6379,8000-8100,8888-8890"
                .to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyPortRange {
    pub(crate) start: u16,
    pub(crate) end: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyPortsConfig {
    pub(crate) exact_ports: Vec<u16>,
    pub(crate) ranges: Vec<LegacyPortRange>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProcessInfo {
    pub(crate) port: u16,
    pub(crate) pid: u32,
    pub(crate) name: String,
    pub(crate) address: String,
}

#[derive(Debug, Clone)]
pub(crate) struct DockerContainer {
    pub(crate) id: String,
    pub(crate) name: String,
}

#[derive(Debug, Default)]
pub(crate) struct PartialProcess {
    pub(crate) pid: Option<u32>,
    pub(crate) name: Option<String>,
    pub(crate) address: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ConfigSnapshot {
    pub(crate) path: PathBuf,
    pub(crate) ports: BTreeSet<u16>,
}
