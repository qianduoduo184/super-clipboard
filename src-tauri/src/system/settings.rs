use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub recording_enabled: bool,
    pub max_history_items: i64,
    pub retention_days: i64,
    pub global_shortcut: String,
    pub autostart_enabled: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            recording_enabled: true,
            max_history_items: 10_000,
            retention_days: 30,
            global_shortcut: "Ctrl+Shift+V".to_string(),
            autostart_enabled: false,
        }
    }
}

impl AppSettings {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(path)?;
        let settings = serde_json::from_str(&content)?;
        Ok(settings)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}
