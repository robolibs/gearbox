//! `gearbox subscribe`: stream any topic, decoded generically by schema —
//! the one way to tail a topic, whether it's host-level or under a
//! machine. Replaces the old split between `machine sub` and `api sub`.

use std::time::Duration;

use clap::Args as ClapArgs;
use gearbox_api::{Env, topics};

use super::api::print_env;
use super::install_ctrlc;
use crate::ctx::Ctx;
use crate::error::{CliError, Result};

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// A topic, always starting with `/` — a single segment like `/imu`
    /// resolves under --machine; a full path (e.g. `/machines/oxbo/imu`,
    /// `/gearbox/info`) is used exactly as given
    topic: String,
    /// Resolve the topic under this machine (default: the selection)
    #[arg(long)]
    machine: Option<String>,
    /// Dial the machine's own did directly instead of resolving --machine
    /// through the host's directory; works even if the host is down, and
    /// needs no instance at all
    #[arg(long)]
    did: Option<String>,
    /// Stop after this many messages
    #[arg(short = 'n', long)]
    count: Option<usize>,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<()> {
    let Some(leaf) = args.topic.strip_prefix('/') else {
        return Err(CliError::usage(format!(
            "topic `{}` must start with `/`, e.g. `/imu` or `/machines/oxbo/imu`",
            args.topic
        )));
    };
    // A single segment (no further `/`) is a leaf resolved under --machine;
    // anything with more segments is already a full path, used as given.
    let full_topic = if !leaf.is_empty() && !leaf.contains('/') {
        topics::machine_topic(&ctx.machine_ns(args.machine)?, leaf)
    } else {
        args.topic.clone()
    };
    // `--did` dials the machine directly and needs no instance at all; the
    // usual path connects through one to resolve the topic's owner.
    let mut sub = match args.did {
        Some(did) => ctx.bare_client()?.subscribe_env_direct(&did, &full_topic)?,
        None => ctx.client()?.subscribe_env(&full_topic)?,
    };
    let stop_flag = install_ctrlc();
    let mut seen = 0usize;
    loop {
        if stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }
        match sub.recv_timeout(Duration::from_millis(200)) {
            Ok(Some(sample)) => {
                let env = Env::new(sample.header().type_hash, sample.payload().to_vec());
                print_env(ctx, &env);
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
