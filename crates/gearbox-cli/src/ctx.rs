//! Per-invocation state: global flags, config and context files, the
//! resolved target instance, and one lazily connected client.

use std::cell::OnceCell;
use std::time::Duration;

use gearbox_api::registry::{self, RegistryEntry};
use gearbox_api::{Client, IdentitySource, object_kind};
use serde_json::Value;

use crate::error::{CliError, Result};
use crate::paths::{Config, Context};

pub const CLI_IDENTITY: &str = "gearbox-cli";

pub struct Ctx {
    pub instance: Option<String>,
    pub json: bool,
    pub quiet: bool,
    pub timeout: Duration,
    pub config: Config,
    pub context: Context,
    target: OnceCell<RegistryEntry>,
    client: OnceCell<Client>,
}

impl Ctx {
    pub fn new(instance: Option<String>, json: bool, quiet: bool, timeout_s: f64) -> Result<Self> {
        let config = Config::load()?;
        let context = Context::load()?;
        let json = json || config.output.as_deref() == Some("json");
        Ok(Self {
            instance,
            json,
            quiet,
            timeout: Duration::from_secs_f64(timeout_s.max(0.05)),
            config,
            context,
            target: OnceCell::new(),
            client: OnceCell::new(),
        })
    }

    /// The instance this command talks to: flag, `GEARBOX_INSTANCE`, the
    /// context file, the config default, then the only live entry.
    pub fn target(&self) -> Result<&RegistryEntry> {
        if let Some(t) = self.target.get() {
            return Ok(t);
        }
        let wanted = self
            .instance
            .clone()
            .or_else(|| {
                std::env::var("GEARBOX_INSTANCE")
                    .ok()
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| self.context.instance.clone())
            .or_else(|| self.config.instance.clone());
        let entries = registry::list();
        let entry = match wanted {
            Some(name) if name.starts_with("did:key:") => entries
                .iter()
                .find(|e| e.did == name)
                .cloned()
                .unwrap_or_else(|| RegistryEntry {
                    name: name.clone(),
                    did: name.clone(),
                    addr: String::new(),
                    pid: 0,
                    started: String::new(),
                    version: String::new(),
                    log: None,
                }),
            Some(name) => entries
                .iter()
                .find(|e| e.name == name)
                .cloned()
                .ok_or_else(|| {
                    CliError::no_instance(format!(
                        "no running instance named `{name}` in {}{}",
                        registry::registry_dir().display(),
                        candidates(&entries)
                    ))
                })?,
            None => match entries.as_slice() {
                [only] => only.clone(),
                [] => {
                    return Err(CliError::no_instance(format!(
                        "no running gearbox instance in {}; start one with `gearbox run`",
                        registry::registry_dir().display()
                    )));
                }
                [first, ..] => {
                    eprintln!(
                        "several instances are running; using `{}` (pass --instance or `gearbox instance use` to pick another){}",
                        first.name,
                        candidates(entries.as_slice())
                    );
                    first.clone()
                }
            },
        };
        let _ = self.target.set(entry);
        Ok(self.target.get().expect("set above"))
    }

    pub fn client(&self) -> Result<&Client> {
        if let Some(c) = self.client.get() {
            return Ok(c);
        }
        let target = self.target()?.clone();
        ensure_cli_did()?;
        let client = Client::connect(
            &target.did,
            IdentitySource::Name(CLI_IDENTITY.to_string()),
            CLI_IDENTITY,
        )?;
        client.wait_ready(self.timeout).map_err(|err| {
            CliError::timeout(format!(
                "instance `{}` ({}) did not answer /gearbox/info within {:.1}s: {err}",
                target.name,
                target.did,
                self.timeout.as_secs_f64()
            ))
        })?;
        let _ = self.client.set(client);
        Ok(self.client.get().expect("set above"))
    }

    /// A bare client with no bootstrap host — for a command addressing a
    /// machine directly by its own did, which needs no gearbox instance to
    /// be resolvable, reachable, or even running.
    pub fn bare_client(&self) -> Result<Client> {
        ensure_cli_did()?;
        Ok(Client::bare(
            IdentitySource::Name(CLI_IDENTITY.to_string()),
            CLI_IDENTITY,
        )?)
    }

    /// The machine a `machine` leaf acts on: the argument, else the selection.
    pub fn machine_id(&self, arg: Option<String>) -> Result<String> {
        if let Some(machine_id) = arg {
            return Ok(machine_id);
        }
        match &self.context.selection {
            Some(sel) if sel.kind == object_kind::name(object_kind::MACHINE) => Ok(sel.id.clone()),
            _ => Err(CliError::usage(
                "no machine given and none selected; pass MACHINE or run `gearbox select machine MACHINE`",
            )),
        }
    }

    /// Print JSON when asked, otherwise run the human printer.
    pub fn emit(&self, json: impl FnOnce() -> Value, human: impl FnOnce()) {
        if self.json {
            println!("{}", crate::out::pretty_json(&json()));
        } else if !self.quiet {
            human();
        }
    }

    /// One-line action result, or only the id under `--quiet`.
    pub fn done(&self, id: &str, message: &str, json: impl FnOnce() -> Value) {
        if self.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json()).unwrap_or_default()
            );
        } else if self.quiet {
            println!("{id}");
        } else {
            println!("{message}");
        }
    }
}

fn candidates(entries: &[RegistryEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let names: Vec<String> = entries
        .iter()
        .map(|e| format!("{} ({})", e.name, e.did))
        .collect();
    format!("\n  candidates: {}", names.join(", "))
}

/// The CLI's did, written where the sim reads it so it is always allowed.
pub fn ensure_cli_did() -> Result<String> {
    let path = gearbox_api::host::cli_did_path();
    let key = agentio::resolve_identity(&IdentitySource::Name(CLI_IDENTITY.to_string()))?;
    let did = agentio::did_key::endpoint_to_did_key(&key.public())?;
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    if current.trim() != did {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, format!("{did}\n"))?;
    }
    Ok(did)
}

pub fn cli_key_path() -> std::path::PathBuf {
    agentio::default_keys_dir()
        .join("name")
        .join(format!("{CLI_IDENTITY}.key"))
}
