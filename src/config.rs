use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub llm: LlmConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub open_browser_on_start: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LlmConfig {
    pub api_base_url: String,
    pub api_key: String,
    pub default_model: String,
    pub context_window: usize,
    pub temperature: f32,
    pub top_p: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UiConfig {
    pub theme: String,
    pub stream_responses: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 8080,
                open_browser_on_start: true,
            },
            llm: LlmConfig {
                api_base_url: "http://127.0.0.1:5001/v1".to_string(),
                api_key: "not-needed".to_string(),
                default_model: "local-model".to_string(),
                context_window: 4096,
                temperature: 0.7,
                top_p: 0.9,
            },
            ui: UiConfig {
                theme: "dark".to_string(),
                stream_responses: true,
            },
        }
    }
}

impl AppConfig {
    pub fn load_or_create<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let path = path.as_ref();
        if path.exists() {
            let content = fs::read_to_string(path)?;
            let config: AppConfig = serde_yaml::from_str(&content)?;
            Ok(config)
        } else {
            let default_config = AppConfig::default();
            let yaml_str = serde_yaml::to_string(&default_config)?;
            fs::write(path, yaml_str)?;
            Ok(default_config)
        }
    }
}