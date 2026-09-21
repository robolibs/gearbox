//! `gearbox peer`: browse the live topic directory and stream any topic in
//! it — the one way to see what's actually running, host-level or under a
//! machine, and the general-purpose home for more peer-inspection verbs.

use std::io::{IsTerminal, Write};
use std::time::Duration;

use clap::{Args as ClapArgs, Subcommand};
use gearbox_api::Env;
use gearbox_api::agentio::{ExchangeKind, TopicEntry};
use serde_json::json;

use super::api::{print_env, render_env};
use super::install_ctrlc;
use crate::ctx::Ctx;
use crate::error::{CliError, Result};
use crate::out::Table;

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Topics the instance's directory currently knows about, optionally
    /// only those at or under PREFIX
    List {
        /// e.g. `/machines/kubota`; omit to list everything
        prefix: Option<String>,
    },
    /// Stream a topic, decoded generically by schema — a prefix with no
    /// topic of its own (e.g. `/machines/kubota`) lists its children
    /// instead of erroring
    Sub {
        /// The topic, e.g. `/machines/oxbo/imu` or `/gearbox/info`
        topic: String,
        /// Dial this did directly instead of resolving the topic through
        /// the host's directory; works even if the host is down, and needs
        /// no instance at all
        #[arg(long)]
        did: Option<String>,
        /// Stop after this many messages
        #[arg(short = 'n', long)]
        count: Option<usize>,
        /// Redraw each message over the last instead of scrolling the
        /// terminal with one block per message; ignored when stdout isn't
        /// a terminal
        #[arg(long)]
        inline: bool,
    },
}

pub fn run(ctx: &Ctx, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::List { prefix } => list(ctx, prefix.as_deref()),
        Cmd::Sub {
            topic,
            did,
            count,
            inline,
        } => sub(ctx, &topic, did.as_deref(), count, inline),
    }
}

fn list(ctx: &Ctx, prefix: Option<&str>) -> Result<()> {
    let entries = matching_topics(ctx, prefix)?;
    print_topics(ctx, &entries);
    Ok(())
}

fn sub(ctx: &Ctx, topic: &str, did: Option<&str>, count: Option<usize>, inline: bool) -> Result<()> {
    if !topic.starts_with('/') {
        return Err(CliError::usage(format!(
            "topic `{topic}` must start with `/`, e.g. `/machines/oxbo/imu`"
        )));
    }
    // `--did` dials the peer directly and needs no instance at all; the
    // usual path connects through one to resolve the topic's owner.
    let mut subscriber = match did {
        Some(did) => ctx.bare_client()?.subscribe_env_direct(did, topic)?,
        None => match ctx.client()?.subscribe_env(topic) {
            Ok(subscriber) => subscriber,
            // Not a live topic itself — before giving up, check whether it's
            // a prefix with live topics under it (e.g. `/machines/kubota`
            // instead of `/machines/kubota/odom`) and list those instead of
            // erroring on what was really just an incomplete path.
            Err(err) => {
                let children = matching_topics(ctx, Some(topic))?
                    .into_iter()
                    .filter(|entry| entry.topic() != topic)
                    .collect::<Vec<_>>();
                if children.is_empty() {
                    return Err(err.into());
                }
                print_topics(ctx, &children);
                return Ok(());
            }
        },
    };
    let inline = inline && std::io::stdout().is_terminal();
    let stop_flag = install_ctrlc();
    let mut seen = 0usize;
    let mut previous_lines = 0usize;
    loop {
        if stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }
        match subscriber.recv_timeout(Duration::from_millis(200)) {
            Ok(Some(sample)) => {
                let env = Env::new(sample.header().type_hash, sample.payload().to_vec());
                if inline {
                    previous_lines = redraw(previous_lines, &render_env(ctx, &env));
                } else {
                    print_env(ctx, &env);
                }
                seen += 1;
                if count.is_some_and(|n| seen >= n) {
                    return Ok(());
                }
            }
            Ok(None) => continue,
            Err(peerbus::Error::Lagged { .. }) => continue,
            Err(err) => return Err(agentio::Error::from(err).into()),
        }
    }
}

/// Erases the last `previous_lines` lines this same command printed, then
/// prints `text` in their place; returns the line count to pass back in on
/// the next call.
fn redraw(previous_lines: usize, text: &str) -> usize {
    let mut out = std::io::stdout();
    if previous_lines > 0 {
        // Cursor Previous Line ×N, then clear from there to end of screen.
        let _ = write!(out, "\x1b[{previous_lines}F\x1b[J");
    }
    let _ = writeln!(out, "{text}");
    let _ = out.flush();
    text.lines().count()
}

/// Live topics at or under `prefix` (every known topic, with none),
/// refreshed from the instance's directory first. Shared by `peer list`
/// and by `peer sub`'s listing fallback when an exact topic won't resolve.
fn matching_topics(ctx: &Ctx, prefix: Option<&str>) -> Result<Vec<TopicEntry>> {
    let client = ctx.client()?;
    // Best-effort: list from whatever the local cache already has rather
    // than failing the whole command if the instance is momentarily slow
    // to answer a resync.
    let _ = client.agent.reconcile_now();
    let mut entries = client.agent.directory().all_entries();
    if let Some(prefix) = prefix {
        let prefix = prefix.trim_end_matches('/');
        entries.retain(|entry| {
            let topic = entry.topic();
            topic == prefix || topic.starts_with(&format!("{prefix}/"))
        });
    }
    entries.sort_by(|a, b| a.topic().cmp(b.topic()));
    Ok(entries)
}

fn print_topics(ctx: &Ctx, entries: &[TopicEntry]) {
    ctx.emit(
        || {
            json!(
                entries
                    .iter()
                    .map(|entry| json!({
                        "topic": entry.topic(),
                        "mode": exchange_label(entry.exchange()),
                    }))
                    .collect::<Vec<_>>()
            )
        },
        || {
            if entries.is_empty() {
                println!("no topics");
                return;
            }
            let mut table = Table::new(&["TOPIC", "MODE"]);
            for entry in entries {
                table.row(vec![
                    entry.topic().to_string(),
                    exchange_label(entry.exchange()).to_string(),
                ]);
            }
            table.print();
        },
    );
}

fn exchange_label(kind: ExchangeKind) -> &'static str {
    match kind {
        ExchangeKind::PubSub => "pubsub",
        ExchangeKind::ReqRes => "reqres",
        ExchangeKind::QueAns => "queans",
        ExchangeKind::PutAck => "putack",
        ExchangeKind::Pip => "pip",
    }
}
