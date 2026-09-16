//! `gearbox peer`: browse the live topic directory and stream any topic in
//! it — the general-purpose window into what's actually running, rather
//! than what one specific command already knows to ask about. More
//! peer-inspection verbs land here over time.

use clap::{Args as ClapArgs, Subcommand};
use gearbox_api::agentio::{ExchangeKind, TopicEntry};
use serde_json::json;

use super::subscribe;
use crate::ctx::Ctx;
use crate::error::Result;
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
    /// Stream a topic — same as `gearbox subscribe`
    Sub {
        /// The topic, e.g. `/machines/oxbo/imu` or `/gearbox/info`
        topic: String,
        #[arg(long)]
        did: Option<String>,
        /// Stop after this many messages
        #[arg(short = 'n', long)]
        count: Option<usize>,
        /// Redraw each message over the last instead of scrolling
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
        } => subscribe::run(
            ctx,
            subscribe::Args {
                topic,
                did,
                count,
                inline,
            },
        ),
    }
}

fn list(ctx: &Ctx, prefix: Option<&str>) -> Result<()> {
    let entries = matching_topics(ctx, prefix)?;
    print_topics(ctx, &entries);
    Ok(())
}

/// Live topics at or under `prefix` (every known topic, with none),
/// refreshed from the instance's directory first. Shared by `peer list`
/// and by `subscribe`'s listing fallback when an exact topic won't
/// resolve.
pub(crate) fn matching_topics(ctx: &Ctx, prefix: Option<&str>) -> Result<Vec<TopicEntry>> {
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

pub(crate) fn print_topics(ctx: &Ctx, entries: &[TopicEntry]) {
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
