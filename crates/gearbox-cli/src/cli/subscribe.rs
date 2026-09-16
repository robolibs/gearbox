//! `gearbox subscribe`: stream any topic, decoded generically by schema —
//! the one way to tail a topic, whether it's host-level or under a
//! machine. Replaces the old split between `machine sub` and `api sub`.

use std::io::{IsTerminal, Write};
use std::time::Duration;

use clap::Args as ClapArgs;
use gearbox_api::Env;

use super::api::{print_env, render_env};
use super::install_ctrlc;
use crate::ctx::Ctx;
use crate::error::{CliError, Result};

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// The topic, e.g. `/machines/oxbo/imu` or `/gearbox/info`
    pub(crate) topic: String,
    /// Dial this did directly instead of resolving the topic through the
    /// host's directory; works even if the host is down, and needs no
    /// instance at all
    #[arg(long)]
    pub(crate) did: Option<String>,
    /// Stop after this many messages
    #[arg(short = 'n', long)]
    pub(crate) count: Option<usize>,
    /// Redraw each message over the last instead of scrolling the terminal
    /// with one block per message; ignored when stdout isn't a terminal
    #[arg(long)]
    pub(crate) inline: bool,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<()> {
    if !args.topic.starts_with('/') {
        return Err(CliError::usage(format!(
            "topic `{}` must start with `/`, e.g. `/machines/oxbo/imu`",
            args.topic
        )));
    }
    // `--did` dials the peer directly and needs no instance at all; the
    // usual path connects through one to resolve the topic's owner.
    let mut sub = match args.did {
        Some(did) => ctx.bare_client()?.subscribe_env_direct(&did, &args.topic)?,
        None => match ctx.client()?.subscribe_env(&args.topic) {
            Ok(sub) => sub,
            // Not a live topic itself — before giving up, check whether it's
            // a prefix with live topics under it (e.g. `/machines/kubota`
            // instead of `/machines/kubota/odom`) and list those instead of
            // erroring on what was really just an incomplete path.
            Err(err) => {
                let children = super::peer::matching_topics(ctx, Some(&args.topic))?
                    .into_iter()
                    .filter(|entry| entry.topic() != args.topic)
                    .collect::<Vec<_>>();
                if children.is_empty() {
                    return Err(err.into());
                }
                super::peer::print_topics(ctx, &children);
                return Ok(());
            }
        },
    };
    let inline = args.inline && std::io::stdout().is_terminal();
    let stop_flag = install_ctrlc();
    let mut seen = 0usize;
    let mut previous_lines = 0usize;
    loop {
        if stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }
        match sub.recv_timeout(Duration::from_millis(200)) {
            Ok(Some(sample)) => {
                let env = Env::new(sample.header().type_hash, sample.payload().to_vec());
                if inline {
                    previous_lines = redraw(previous_lines, &render_env(ctx, &env));
                } else {
                    print_env(ctx, &env);
                }
                seen += 1;
                if args.count.is_some_and(|n| seen >= n) {
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
