//! Where the CLI keeps its files: config under XDG config, context and logs
//! under XDG state. The sim's registry lives in `gearbox_api::registry`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{CliError, Result};

fn xdg(var: &str, fallback: &[&str]) -> PathBuf {
    if let Some(dir) = std::env::var_os(var).filter(|v| !v.is_empty()) {
        return PathBuf::from(dir).join("gearbox");
    }
    let mut base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    for part in fallback {
        base = base.join(part);
    }
    base.join("gearbox")
}

pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", &[".config"])
}

pub fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", &[".local", "state"])
}

pub fn logs_dir() -> PathBuf {
    state_dir().join("logs")
}

pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn context_file() -> PathBuf {
    state_dir().join("context.toml")
}

/// Extra peers the sim allows on launch, one did per line.
pub fn allow_file() -> PathBuf {
    config_dir().join("allow")
}

fn write_atomic(path: &PathBuf, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub instance: Option<String>,
    pub output: Option<String>,
    pub asset_root: Option<String>,
    pub sim: Option<String>,
}

impl Config {
    pub const KEYS: [&'static str; 4] = ["instance", "output", "asset_root", "sim"];

    pub fn load() -> Result<Self> {
        match std::fs::read_to_string(config_file()) {
            Ok(text) => Ok(toml::from_str(&text)?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn save(&self) -> Result<()> {
        let text = toml::to_string_pretty(self).map_err(|e| CliError::error(e.to_string()))?;
        write_atomic(&config_file(), &text)
    }

    pub fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(match key {
            "instance" => self.instance.clone(),
            "output" => self.output.clone(),
            "asset_root" => self.asset_root.clone(),
            "sim" => self.sim.clone(),
            _ => return Err(CliError::usage(format!("unknown config key `{key}`"))),
        })
    }

    pub fn set(&mut self, key: &str, value: Option<String>) -> Result<()> {
        match key {
            "instance" => self.instance = value,
            "output" => self.output = value,
            "asset_root" => self.asset_root = value,
            "sim" => self.sim = value,
            _ => return Err(CliError::usage(format!("unknown config key `{key}`"))),
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SelectionCtx {
    pub kind: String,
    pub id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Context {
    pub instance: Option<String>,
    pub selection: Option<SelectionCtx>,
    pub world: Option<String>,
    #[serde(default)]
    pub manifests: Vec<String>,
}

impl Context {
    pub fn load() -> Result<Self> {
        match std::fs::read_to_string(context_file()) {
            Ok(text) => Ok(toml::from_str(&text)?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn save(&self) -> Result<()> {
        let text = toml::to_string_pretty(self).map_err(|e| CliError::error(e.to_string()))?;
        write_atomic(&context_file(), &text)
    }
}

pub fn read_allow_file() -> Vec<String> {
    std::fs::read_to_string(allow_file())
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub fn write_allow_file(dids: &[String]) -> Result<()> {
    let mut text = dids.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    write_atomic(&allow_file(), &text)
}
