//! `gearbox subscribe`: stream any topic, decoded generically by schema —
//! the one way to tail a topic, whether it's host-level or under a
//! machine. Replaces the old split between `machine sub` and `api sub`.

use std::time::Duration;

use clap::Args as ClapArgs;
use gearbox_api::Env;

use super::api::print_env;
use super::install_ctrlc;
use crate::ctx::Ctx;
use crate::error::{CliError, Result};

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// The topic, e.g. `/machines/oxbo/imu` or `/gearbox/info`
    topic: String,
    /// Dial this did directly instead of resolving the topic through the
    /// host's directory; works even if the host is down, and needs no
    /// instance at all
    #[arg(long)]
    did: Option<String>,
    /// Stop after this many messages
    #[arg(short = 'n', long)]
    count: Option<usize>,
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
        None => ctx.client()?.subscribe_env(&args.topic)?,
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
