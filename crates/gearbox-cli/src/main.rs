//! `gearbox` — the command-line front door for the simulator.
//!
//! Eleven command groups; `main` only parses and dispatches. Anything that
//! is not a core group runs `gearbox-<name>` from `PATH`, git-style.

mod cli;
mod ctx;
mod error;
mod out;
mod paths;

use std::ffi::OsString;

use clap::{CommandFactory, Parser, Subcommand};

use crate::ctx::Ctx;
use crate::error::{CliError, EXIT_USAGE, Result};

#[derive(Parser, Debug)]
#[command(
    name = "gearbox",
    version,
    about = "Launch, inspect and drive the gearbox simulator",
    long_about = None,
    propagate_version = true,
    after_help = extensions_help()
)]
pub struct Cli {
    /// Instance name or did:key to talk to (else GEARBOX_INSTANCE, context, the only live one)
    #[arg(short, long, global = true, env = "GEARBOX_INSTANCE")]
    instance: Option<String>,

    /// Print the wire response as JSON
    #[arg(long, global = true)]
    json: bool,

    /// Print ids only
    #[arg(long, short, global = true)]
    quiet: bool,

    /// Seconds to wait for a request
    #[arg(long, global = true, default_value_t = 5.0)]
    timeout: f64,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Launch a simulator instance
    Run(cli::run::Args),
    /// Find, inspect and stop instances; pick the default
    Instance(cli::instance::Args),
    /// Put things into the scene
    Spawn(cli::spawn::Args),
    /// Take things out of the scene
    Clear(cli::clear::Args),
    /// Clock, listing, poses, events
    Scene(cli::scene::Args),
    /// Choose the thing later commands act on
    Select(cli::select::Args),
    /// Inspect and drive machines
    Machine(cli::machine::Args),
    /// Browse the live topic directory, or stream one of its topics
    Peer(cli::peer::Args),
    /// Identities, allowlists, sessions
    Access(cli::access::Args),
    /// Raw topic access, schemas, diagnostics
    Api(cli::api::Args),
    /// CLI config, keys, completions, doctor
    Env(cli::env::Args),
    /// Run `gearbox-<name>` from PATH
    #[command(external_subcommand)]
    External(Vec<OsString>),
}

fn main() {
    let cli = Cli::parse();
    let code = match dispatch(cli) {
        Ok(()) => 0,
        Err(err) => {
            if !err.message.is_empty() {
                eprintln!("gearbox: {}", err.message);
            }
            err.code
        }
    };
    std::process::exit(code);
}

fn dispatch(cli: Cli) -> Result<()> {
    let ctx = Ctx::new(cli.instance, cli.json, cli.quiet, cli.timeout)?;
    match cli.command {
        Command::Run(args) => cli::run::run(&ctx, args),
        Command::Instance(args) => cli::instance::run(&ctx, args),
        Command::Spawn(args) => cli::spawn::run(&ctx, args),
        Command::Clear(args) => cli::clear::run(&ctx, args),
        Command::Scene(args) => cli::scene::run(&ctx, args),
        Command::Select(args) => cli::select::run(&ctx, args),
        Command::Machine(args) => cli::machine::run(&ctx, args),
        Command::Peer(args) => cli::peer::run(&ctx, args),
        Command::Access(args) => cli::access::run(&ctx, args),
        Command::Api(args) => cli::api::run(&ctx, args),
        Command::Env(args) => cli::env::run(&ctx, args),
        Command::External(argv) => cli::extension::run(&ctx, argv),
    }
}

pub fn command() -> clap::Command {
    Cli::command()
}

fn extensions_help() -> String {
    let found = cli::extension::installed();
    if found.is_empty() {
        return String::new();
    }
    let mut text = String::from("Extensions found on PATH:\n");
    for name in found {
        text.push_str(&format!("  {name}\n"));
    }
    text
}

pub fn usage(message: impl Into<String>) -> CliError {
    CliError::new(EXIT_USAGE, message)
}
