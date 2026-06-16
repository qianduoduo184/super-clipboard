use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavFiltersConfig {
    pub visible: Vec<String>,
}

impl Default for NavFiltersConfig {
    fn default() -> Self {
        Self {
            visible: vec![
                "all".to_string(),
                "favorites".to_string(),
                "text".to_string(),
                "image".to_string(),
                "files".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub recording_enabled: bool,
    pub max_history_items: i64,
    pub retention_days: i64,
    pub global_shortcut: String,
    pub autostart_enabled: bool,
    #[serde(default = "default_preview_enabled")]
    pub preview_enabled: bool,
    #[serde(default)]
    pub theme_mode: ThemeMode,
    #[serde(default)]
    pub auto_update_enabled: bool,
    #[serde(default)]
    pub last_update_check_date: Option<String>,
    #[serde(default)]
    pub nav_filters_config: NavFiltersConfig,
    #[serde(default)]
    pub custom_data_dir: Option<String>,
    #[serde(default)]
    pub custom_log_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Light,
    Dark,
}

impl Default for ThemeMode {
    fn default() -> Self {
        Self::Light
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            recording_enabled: true,
            max_history_items: 10_000,
            retention_days: 30,
            global_shortcut: "Ctrl+Shift+V".to_string(),
            autostart_enabled: false,
            preview_enabled: true,
            theme_mode: ThemeMode::Light,
            auto_update_enabled: false,
            last_update_check_date: None,
            nav_filters_config: NavFiltersConfig::default(),
            custom_data_dir: None,
            custom_log_dir: None,
        }
    }
}

fn default_preview_enabled() -> bool {
    true
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
