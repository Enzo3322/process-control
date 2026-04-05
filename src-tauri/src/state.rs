use std::{collections::BTreeSet, path::PathBuf, sync::Mutex};

use crate::models::ConfigSnapshot;

pub(crate) struct AppState {
    config: Mutex<ConfigSnapshot>,
}

impl AppState {
    pub(crate) fn new(config: ConfigSnapshot) -> Self {
        Self {
            config: Mutex::new(config),
        }
    }

    pub(crate) fn config_path(&self) -> Result<PathBuf, String> {
        self.config
            .lock()
            .map_err(|_| "Falha ao acessar o cache de configuracao.".to_string())
            .map(|snapshot| snapshot.path.clone())
    }

    pub(crate) fn configured_ports(&self) -> Result<BTreeSet<u16>, String> {
        self.config
            .lock()
            .map_err(|_| "Falha ao acessar o cache de configuracao.".to_string())
            .map(|snapshot| snapshot.ports.clone())
    }

    pub(crate) fn replace_config(&self, config: ConfigSnapshot) -> Result<(), String> {
        let mut snapshot = self
            .config
            .lock()
            .map_err(|_| "Falha ao atualizar o cache de configuracao.".to_string())?;
        *snapshot = config;
        Ok(())
    }
}
