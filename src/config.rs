//! Configuration file handling. The TOML layout is unchanged from the Python
//! version so existing `config.toml` files keep working.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_MODEL: &str = "google:gemini-3.8-flash";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub allow_excluding_changes: bool,
    pub watermark: bool,
    pub model: ModelConfig,
    pub context_formatting: ContextFormattingConfig,
    pub confirmations: Confirmations,
    pub init: InitConfig,
    pub api_keys: BTreeMap<String, String>,
}

/// Which interface actions ask for confirmation. Pressing `Y` in a dialog
/// turns that prompt off; `vibegit config set confirmations.<name> true` turns it back on.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Confirmations {
    pub commit_all: bool,
    pub commit_all_and_push: bool,
    pub push: bool,
    pub rerun: bool,
    pub quit: bool,
    pub unstage: bool,
    /// Offer to derive `.vibegitrules` from the history when the file is missing.
    pub derive_rules: bool,
}

impl Default for Confirmations {
    fn default() -> Self {
        Self {
            commit_all: true,
            commit_all_and_push: true,
            push: true,
            rerun: true,
            quit: true,
            unstage: true,
            derive_rules: true,
        }
    }
}

/// How much history `vibegit init` analyses to derive `.vibegitrules`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct InitConfig {
    /// Number of recent non-merge commits to analyse.
    pub commits: usize,
    /// Files shown per commit; the rest are listed by name only.
    pub max_files_per_commit: usize,
    /// Diff lines shown per file.
    pub max_lines_per_file: usize,
    /// Older commits are dropped once the rendered history exceeds this size.
    pub max_total_kb: u64,
}

impl Default for InitConfig {
    fn default() -> Self {
        Self {
            commits: 20,
            max_files_per_commit: 8,
            max_lines_per_file: 20,
            max_total_kb: 150,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            allow_excluding_changes: true,
            watermark: true,
            model: ModelConfig::default(),
            context_formatting: ContextFormattingConfig::default(),
            confirmations: Confirmations::default(),
            init: InitConfig::default(),
            api_keys: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModelConfig {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            name: DEFAULT_MODEL.to_string(),
            temperature: None,
            base_url: None,
            api_key: None,
            model_provider: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ContextFormattingConfig {
    pub include_active_branch: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncate_lines: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_latest_commits: Option<usize>,
    /// Files larger than this are listed with a placeholder instead of their content.
    pub max_file_size_kb: u64,
}

impl Default for ContextFormattingConfig {
    fn default() -> Self {
        Self {
            include_active_branch: true,
            truncate_lines: Some(240),
            include_latest_commits: Some(5),
            max_file_size_kb: 512,
        }
    }
}

/// Where the configuration lives (`VIBEGIT_CONFIG_PATH` overrides the platform default).
pub fn config_path() -> PathBuf {
    if let Some(p) = std::env::var_os("VIBEGIT_CONFIG_PATH") {
        return PathBuf::from(p);
    }
    platform_config_dir().join("config.toml")
}

/// Mirrors Python's `platformdirs.user_config_dir("vibegit")`.
fn platform_config_dir() -> PathBuf {
    #[allow(deprecated)]
    let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home.join("Library")
            .join("Application Support")
            .join("vibegit")
    } else if cfg!(target_os = "windows") {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Local"));
        base.join("vibegit").join("vibegit")
    } else {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        base.join("vibegit")
    }
}

impl Config {
    /// Loads the configuration. Returns `(config, existed)`.
    pub fn load() -> Result<(Self, bool)> {
        Self::load_from(&config_path())
    }

    pub fn load_from(path: &Path) -> Result<(Self, bool)> {
        if !path.exists() {
            return Ok((Self::default(), false));
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("could not read {}", path.display()))?;
        let config: Config =
            toml::from_str(&text).with_context(|| format!("could not parse {}", path.display()))?;
        Ok((config, true))
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&config_path())
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        let text = self.to_toml()?;
        std::fs::write(path, text)
            .with_context(|| format!("could not write {}", path.display()))?;
        Ok(())
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).context("could not serialise configuration")
    }

    /// TOML with secrets replaced by a short prefix, for `config show`.
    pub fn to_masked_toml(&self) -> Result<String> {
        let mut masked = self.clone();
        for v in masked.api_keys.values_mut() {
            *v = mask(v);
        }
        if let Some(k) = masked.model.api_key.as_mut() {
            *k = mask(k);
        }
        masked.to_toml()
    }

    /// Reads a value by dotted path (`model.name`, `api_keys.OPENAI_API_KEY`).
    pub fn get(&self, path: &str) -> Result<Option<toml::Value>> {
        let root = toml::Value::try_from(self).context("could not serialise configuration")?;
        let mut current = &root;
        for part in path.split('.') {
            match current.get(part) {
                Some(v) => current = v,
                None => return Ok(None),
            }
        }
        Ok(Some(current.clone()))
    }

    /// Sets a value by dotted path, coercing the text to the field's type.
    ///
    /// `none`/`null` clears optional fields.
    pub fn set(&mut self, path: &str, raw: &str) -> Result<()> {
        let value = raw.trim();
        let clear = value.eq_ignore_ascii_case("none") || value.eq_ignore_ascii_case("null");
        match path {
            "model.name" => self.model.name = require_text(path, value)?,
            "model.temperature" => {
                self.model.temperature = if clear {
                    None
                } else {
                    Some(
                        value
                            .parse::<f64>()
                            .map_err(|_| bad(path, value, "a number"))?,
                    )
                }
            }
            "model.base_url" => self.model.base_url = optional_text(value, clear),
            "model.api_key" => self.model.api_key = optional_text(value, clear),
            "model.model_provider" => self.model.model_provider = optional_text(value, clear),
            "context_formatting.include_active_branch" => {
                self.context_formatting.include_active_branch = parse_bool(path, value)?
            }
            "context_formatting.truncate_lines" => {
                self.context_formatting.truncate_lines = if clear {
                    None
                } else {
                    Some(
                        value
                            .parse::<usize>()
                            .map_err(|_| bad(path, value, "an integer"))?,
                    )
                }
            }
            "context_formatting.include_latest_commits" => {
                self.context_formatting.include_latest_commits = if clear {
                    None
                } else {
                    Some(
                        value
                            .parse::<usize>()
                            .map_err(|_| bad(path, value, "an integer"))?,
                    )
                }
            }
            "context_formatting.max_file_size_kb" => {
                self.context_formatting.max_file_size_kb = value
                    .parse::<u64>()
                    .map_err(|_| bad(path, value, "an integer"))?
            }
            "allow_excluding_changes" => self.allow_excluding_changes = parse_bool(path, value)?,
            "watermark" => self.watermark = parse_bool(path, value)?,
            "confirmations.commit_all" => self.confirmations.commit_all = parse_bool(path, value)?,
            "confirmations.commit_all_and_push" => {
                self.confirmations.commit_all_and_push = parse_bool(path, value)?
            }
            "confirmations.push" => self.confirmations.push = parse_bool(path, value)?,
            "confirmations.rerun" => self.confirmations.rerun = parse_bool(path, value)?,
            "confirmations.quit" => self.confirmations.quit = parse_bool(path, value)?,
            "confirmations.unstage" => self.confirmations.unstage = parse_bool(path, value)?,
            "confirmations.derive_rules" => {
                self.confirmations.derive_rules = parse_bool(path, value)?
            }
            "init.commits" => {
                self.init.commits = value
                    .parse::<usize>()
                    .map_err(|_| bad(path, value, "an integer"))?
            }
            "init.max_files_per_commit" => {
                self.init.max_files_per_commit = value
                    .parse::<usize>()
                    .map_err(|_| bad(path, value, "an integer"))?
            }
            "init.max_lines_per_file" => {
                self.init.max_lines_per_file = value
                    .parse::<usize>()
                    .map_err(|_| bad(path, value, "an integer"))?
            }
            "init.max_total_kb" => {
                self.init.max_total_kb = value
                    .parse::<u64>()
                    .map_err(|_| bad(path, value, "an integer"))?
            }
            _ => {
                if let Some(key) = path.strip_prefix("api_keys.") {
                    if key.is_empty() || key.contains('.') {
                        bail!("invalid api key name in path {path:?}");
                    }
                    if clear {
                        self.api_keys.remove(key);
                    } else {
                        self.api_keys.insert(key.to_string(), value.to_string());
                    }
                } else {
                    bail!(
                        "unknown configuration path {path:?}. Known paths: {}",
                        KNOWN_PATHS.join(", ")
                    );
                }
            }
        }
        Ok(())
    }
}

pub const KNOWN_PATHS: &[&str] = &[
    "model.name",
    "model.temperature",
    "model.base_url",
    "model.api_key",
    "model.model_provider",
    "context_formatting.include_active_branch",
    "context_formatting.truncate_lines",
    "context_formatting.include_latest_commits",
    "context_formatting.max_file_size_kb",
    "allow_excluding_changes",
    "watermark",
    "confirmations.commit_all",
    "confirmations.commit_all_and_push",
    "confirmations.push",
    "confirmations.rerun",
    "confirmations.quit",
    "confirmations.unstage",
    "confirmations.derive_rules",
    "init.commits",
    "init.max_files_per_commit",
    "init.max_lines_per_file",
    "init.max_total_kb",
    "api_keys.<NAME>",
];

fn mask(secret: &str) -> String {
    if secret.len() <= 8 {
        "********".to_string()
    } else {
        format!("{}…{}", &secret[..4], &secret[secret.len() - 2..])
    }
}

fn bad(path: &str, value: &str, expected: &str) -> anyhow::Error {
    anyhow!("invalid value {value:?} for {path}: expected {expected}")
}

fn require_text(path: &str, value: &str) -> Result<String> {
    if value.is_empty() {
        bail!("{path} cannot be empty");
    }
    Ok(value.to_string())
}

fn optional_text(value: &str, clear: bool) -> Option<String> {
    if clear || value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_bool(path: &str, value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(bad(path, value, "true or false")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_python_layout() {
        let text = r#"allow_excluding_changes = false
watermark = true

[model]
name = "openai:gpt-5.6-terra"
temperature = 0.2

[context_formatting]
include_active_branch = true
truncate_lines = 120
include_latest_commits = 3

[api_keys]
OPENAI_API_KEY = "sk-test-1234567890"
"#;
        let config: Config = toml::from_str(text).unwrap();
        assert_eq!(config.model.name, "openai:gpt-5.6-terra");
        assert_eq!(config.model.temperature, Some(0.2));
        assert!(!config.allow_excluding_changes);
        assert_eq!(config.context_formatting.truncate_lines, Some(120));
        assert_eq!(config.context_formatting.max_file_size_kb, 512);
        let back: Config = toml::from_str(&config.to_toml().unwrap()).unwrap();
        assert_eq!(back, config);
        assert!(config.to_masked_toml().unwrap().contains("sk-t…90"));
    }

    #[test]
    fn get_and_set_by_path() {
        let mut config = Config::default();
        config.set("model.name", "grok:grok-code-fast-1").unwrap();
        config.set("model.temperature", "0.5").unwrap();
        config.set("watermark", "false").unwrap();
        config.set("api_keys.GROK_API_KEY", "abc").unwrap();
        config
            .set("context_formatting.truncate_lines", "none")
            .unwrap();
        assert_eq!(
            config.get("model.name").unwrap().unwrap().as_str(),
            Some("grok:grok-code-fast-1")
        );
        assert_eq!(
            config.get("model.temperature").unwrap().unwrap().as_float(),
            Some(0.5)
        );
        assert_eq!(
            config.get("watermark").unwrap().unwrap().as_bool(),
            Some(false)
        );
        assert_eq!(
            config
                .get("api_keys.GROK_API_KEY")
                .unwrap()
                .unwrap()
                .as_str(),
            Some("abc")
        );
        assert!(config
            .get("context_formatting.truncate_lines")
            .unwrap()
            .is_none());
        assert!(config.set("model.temperature", "warm").is_err());
        assert!(config.set("nope.nope", "1").is_err());
        config.set("api_keys.GROK_API_KEY", "none").unwrap();
        assert!(config.api_keys.is_empty());
        config.set("confirmations.push", "false").unwrap();
        assert!(!config.confirmations.push && config.confirmations.commit_all);
        let toml_text = config.to_toml().unwrap();
        assert!(toml_text.contains("[confirmations]") && toml_text.contains("push = false"));
        let back: Config = toml::from_str(&config.to_toml().unwrap()).unwrap();
        assert_eq!(back.confirmations, config.confirmations);
    }

    #[test]
    fn load_missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let (config, existed) = Config::load_from(&path).unwrap();
        assert!(!existed);
        assert_eq!(config, Config::default());
        config.save_to(&path).unwrap();
        let (_, existed) = Config::load_from(&path).unwrap();
        assert!(existed);
    }
}
