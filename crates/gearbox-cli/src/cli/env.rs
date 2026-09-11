//! `gearbox env`: the CLI's own configuration, keys, completions, versions.

use clap::{Args as ClapArgs, Subcommand};
use clap_complete::Shell;
use serde_json::json;

use crate::ctx::{Ctx, cli_key_path, ensure_cli_did};
use crate::error::Result;
use crate::out;
use crate::paths::{self, Config};

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Current config values
    Show,
    /// Set a config key (instance, output, asset_root, sim); empty VALUE unsets
    Set { key: String, value: Option<String> },
    /// Paths of the config, context, registry and key files
    Path,
    /// Shell completions
    Completions { shell: Shell },
    /// Where identities live
    Keys,
    /// CLI, sim and library versions
    Version,
    /// The command tree as Markdown
    Docs,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::Show => {
            let cfg = &ctx.config;
            ctx.emit(
                || serde_json::to_value(cfg).unwrap_or(json!({})),
                || {
                    let pairs: Vec<(&str, String)> = Config::KEYS
                        .iter()
                        .map(|k| (*k, cfg.get(k).ok().flatten().unwrap_or_default()))
                        .collect();
                    out::kv(&pairs);
                },
            );
            Ok(())
        }
        Cmd::Set { key, value } => {
            let mut cfg = ctx.config.clone();
            cfg.set(&key, value.clone().filter(|v| !v.is_empty()))?;
            cfg.save()?;
            ctx.done(
                &key,
                &format!("{key} = {}", value.unwrap_or_default()),
                || serde_json::to_value(&cfg).unwrap_or(json!({})),
            );
            Ok(())
        }
        Cmd::Path => {
            let pairs = [
                ("config", paths::config_file().display().to_string()),
                ("context", paths::context_file().display().to_string()),
                ("allow", paths::allow_file().display().to_string()),
                ("logs", paths::logs_dir().display().to_string()),
                (
                    "registry",
                    gearbox_api::registry::registry_dir().display().to_string(),
                ),
                (
                    "cli did",
                    gearbox_api::host::cli_did_path().display().to_string(),
                ),
                ("cli key", cli_key_path().display().to_string()),
                ("keys", agentio::default_keys_dir().display().to_string()),
            ];
            ctx.emit(
                || {
                    json!(
                        pairs
                            .iter()
                            .map(|(k, v)| (k.to_string(), json!(v)))
                            .collect::<serde_json::Map<_, _>>()
                    )
                },
                || out::kv(&pairs),
            );
            Ok(())
        }
        Cmd::Completions { shell } => {
            let mut cmd = crate::command();
            clap_complete::generate(shell, &mut cmd, "gearbox", &mut std::io::stdout());
            Ok(())
        }
        Cmd::Keys => {
            let did = ensure_cli_did()?;
            let dir = agentio::default_keys_dir();
            let mut names = Vec::new();
            if let Ok(entries) = std::fs::read_dir(dir.join("name")) {
                for e in entries.flatten() {
                    let n = e.file_name().to_string_lossy().into_owned();
                    if let Some(stem) = n.strip_suffix(".key") {
                        names.push(stem.to_string());
                    }
                }
            }
            names.sort();
            ctx.emit(
                || json!({ "dir": dir.display().to_string(), "cli_did": did, "identities": names }),
                || {
                    out::kv(&[
                        ("keys dir", dir.display().to_string()),
                        ("cli did", did.clone()),
                        ("identities", names.join(", ")),
                    ]);
                },
            );
            Ok(())
        }
        Cmd::Version => version(ctx),
        Cmd::Docs => {
            print!("{}", markdown(&crate::command()));
            Ok(())
        }
    }
}

fn version(ctx: &Ctx) -> Result<()> {
    let cli = env!("CARGO_PKG_VERSION").to_string();
    let sim = ctx
        .target()
        .ok()
        .and_then(|_| ctx.client().ok())
        .and_then(|c| c.info().ok())
        .map(|i| i.props().get("version").unwrap_or_default());
    let pins = pinned_versions();
    ctx.emit(
        || {
            json!({
                "cli": cli,
                "sim": sim,
                "peerbus": pins.get("peerbus"),
                "agentio": pins.get("agentio"),
                "datapod": pins.get("datapod"),
            })
        },
        || {
            out::kv(&[
                ("gearbox", cli.clone()),
                ("sim", sim.clone().unwrap_or_else(|| "not reachable".into())),
                ("peerbus", pins.get("peerbus").cloned().unwrap_or_default()),
                ("agentio", pins.get("agentio").cloned().unwrap_or_default()),
                ("datapod", pins.get("datapod").cloned().unwrap_or_default()),
            ]);
        },
    );
    Ok(())
}

/// Dependency versions as baked into this build.
fn pinned_versions() -> std::collections::HashMap<&'static str, String> {
    let mut m = std::collections::HashMap::new();
    m.insert(
        "datapod",
        datapod::registry::current_wire_format_name().to_string(),
    );
    m.insert(
        "peerbus",
        option_env!("GEARBOX_PEERBUS_REV")
            .unwrap_or("workspace")
            .to_string(),
    );
    m.insert(
        "agentio",
        option_env!("GEARBOX_AGENTIO_REV")
            .unwrap_or("workspace")
            .to_string(),
    );
    m
}

fn markdown(cmd: &clap::Command) -> String {
    let mut out = String::new();
    out.push_str("# gearbox CLI\n\n");
    out.push_str(&format!(
        "{}\n\n",
        cmd.get_about().map(|s| s.to_string()).unwrap_or_default()
    ));
    for group in cmd.get_subcommands() {
        if group.is_hide_set() {
            continue;
        }
        out.push_str(&format!(
            "## `gearbox {}`\n\n{}\n\n",
            group.get_name(),
            group.get_about().map(|s| s.to_string()).unwrap_or_default()
        ));
        let leaves: Vec<_> = group.get_subcommands().collect();
        if leaves.is_empty() {
            out.push_str(&format!("```\n{}\n```\n\n", usage_line(group)));
            continue;
        }
        out.push_str("| Leaf | Does |\n|---|---|\n");
        for leaf in leaves {
            out.push_str(&format!(
                "| `{}` | {} |\n",
                usage_line(leaf).trim_start_matches("gearbox ").trim(),
                leaf.get_about().map(|s| s.to_string()).unwrap_or_default()
            ));
        }
        out.push('\n');
    }
    out
}

fn usage_line(cmd: &clap::Command) -> String {
    let mut c = cmd.clone();
    let usage = c.render_usage().to_string();
    usage
        .lines()
        .next()
        .unwrap_or_default()
        .trim_start_matches("Usage:")
        .trim()
        .to_string()
}
