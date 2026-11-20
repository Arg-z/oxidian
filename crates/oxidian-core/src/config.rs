//! Config aplikasi: `~/.config/oxidian/config.json`.
//! Disimpan DI LUAR vault supaya vault tetap Markdown murni dan
//! tetap kompatibel dibuka Obsidian asli.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub recent_vaults: Vec<String>,
    #[serde(default)]
    pub last_vault: Option<String>,
    #[serde(default)]
    pub last_note: Option<String>,
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Port MCP server (localhost) — null/absen = matikan MCP.
    #[serde(default = "default_mcp_port")]
    pub mcp_port: Option<u16>,
}

fn default_theme() -> String {
    "dark".into()
}

fn default_mcp_port() -> Option<u16> {
    Some(8321)
}

impl Default for Config {
    fn default() -> Self {
        Config {
            recent_vaults: Vec::new(),
            last_vault: None,
            last_note: None,
            theme: default_theme(),
            mcp_port: default_mcp_port(),
        }
    }
}

pub fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/oxidian/config.json")
}

impl Config {
    pub fn load() -> Config {
        std::fs::read_to_string(config_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = config_path();
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&p, serde_json::to_string_pretty(self)?)
            .map_err(std::io::Error::other)
    }

    /// Catat vault yang baru dibuka (pindah ke depan, maks 10).
    pub fn push_recent_vault(&mut self, v: &str) {
        self.recent_vaults.retain(|x| x != v);
        self.recent_vaults.insert(0, v.to_string());
        self.recent_vaults.truncate(10);
        self.last_vault = Some(v.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_dan_default() {
        let mut c = Config::default();
        assert_eq!(c.theme, "dark");
        c.push_recent_vault("/tmp/v1");
        c.push_recent_vault("/tmp/v2");
        c.push_recent_vault("/tmp/v1"); // dedup, pindah depan
        assert_eq!(c.recent_vaults, vec!["/tmp/v1", "/tmp/v2"]);
        assert_eq!(c.last_vault.as_deref(), Some("/tmp/v1"));
        let json = serde_json::to_string(&c).unwrap();
        let c2: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(c2.recent_vaults, c.recent_vaults);
    }

    #[test]
    fn load_file_kosong_tidak_panic() {
        // config lama/rusak → fallback default tanpa error
        let c: Config = serde_json::from_str("{bukan json").unwrap_or_default();
        assert!(c.recent_vaults.is_empty());
    }
}
