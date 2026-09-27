use crate::CoreError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default theme: dark (Catppuccin Mocha).
fn default_dark_mode() -> bool {
    true
}

/// User configuration persisted in `~/.config/rustidian/config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Absolute path to the vault folder.
    pub vault_path: PathBuf,
    /// Whether the dark (Catppuccin Mocha) theme is active.  `false` selects
    /// the light (Catppuccin Latte) theme.
    #[serde(default = "default_dark_mode")]
    pub dark_mode: bool,
}

impl Config {
    /// Return the path of the config file, creating parent directories if needed.
    pub fn config_path() -> Result<PathBuf, CoreError> {
        let base = dirs::config_dir()
            .ok_or_else(|| CoreError::Config("cannot determine config directory".into()))?;
        Ok(base.join("rustidian").join("config.toml"))
    }

    /// Load configuration from disk, or return a sensible default (vault in `~/Notes`).
    pub fn load() -> Result<Self, CoreError> {
        let path = Self::config_path()?;
        if !path.exists() {
            let default_vault = dirs::home_dir()
                .ok_or_else(|| CoreError::Config("cannot determine home directory".into()))?
                .join("Notes");
            return Ok(Config {
                vault_path: default_vault,
                dark_mode: default_dark_mode(),
            });
        }
        let raw = std::fs::read_to_string(&path)?;
        toml::from_str(&raw).map_err(|e| CoreError::Config(e.to_string()))
    }

    /// Persist configuration to disk.
    pub fn save(&self) -> Result<(), CoreError> {
        let path = Self::config_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = toml::to_string_pretty(self).map_err(|e| CoreError::Config(e.to_string()))?;
        std::fs::write(&path, raw)?;
        Ok(())
    }

    /// Update the vault path and immediately persist.
    pub fn set_vault_path(&mut self, path: impl AsRef<Path>) -> Result<(), CoreError> {
        self.vault_path = path.as_ref().to_path_buf();
        self.save()
    }

    /// Update the theme selection and immediately persist.
    pub fn set_dark_mode(&mut self, dark: bool) -> Result<(), CoreError> {
        self.dark_mode = dark;
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_without_theme_field_defaults_to_dark() {
        let raw = "vault_path = \"/tmp/notes\"\n";
        let config: Config = toml::from_str(raw).unwrap();
        assert!(config.dark_mode);
    }

    #[test]
    fn config_roundtrips_theme() {
        let config = Config {
            vault_path: PathBuf::from("/tmp/notes"),
            dark_mode: false,
        };
        let raw = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&raw).unwrap();
        assert!(!parsed.dark_mode);
    }
}
