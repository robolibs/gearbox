//! `gearbox clear`: take things out of the scene.

use std::io::{IsTerminal, Write};

use clap::{Args as ClapArgs, Subcommand};
use gearbox_api::{Selection, clear_scope};
use serde_json::json;

use super::check;
use crate::ctx::Ctx;
use crate::error::{CliError, Result};

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    /// Skip the confirmation for `clear` / `clear all`
    #[arg(short = 'y', long, global = true)]
    yes: bool,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Everything: machines, props, markers (the UI button)
    All,
    /// Despawn every machine, keep props and markers
    Machines,
    /// Props only
    Props,
    /// Markers only
    Markers,
    /// One loaded USD by id
    Usd { id: String },
    /// One marker by id
    Marker { id: String },
    /// Drop the CLI selection and the UI highlight
    Selection,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<()> {
    let cmd = args.cmd.unwrap_or(Cmd::All);
    match cmd {
        Cmd::All => {
            if !args.yes && !confirm(ctx, "clear the whole scene?")? {
                return Err(CliError::new(1, "aborted"));
            }
            scope(ctx, clear_scope::ALL)
        }
        Cmd::Machines => scope(ctx, clear_scope::MACHINES),
        Cmd::Props => scope(ctx, clear_scope::PROPS),
        Cmd::Markers => scope(ctx, clear_scope::MARKERS),
        Cmd::Usd { id } => {
            check(ctx.client()?.delete(&id)?, &format!("delete {id}"))?;
            ctx.done(&id, &format!("deleted {id}"), || json!({ "id": id }));
            Ok(())
        }
        Cmd::Marker { id } => {
            check(
                ctx.client()?.marker_delete(&id)?,
                &format!("delete marker {id}"),
            )?;
            ctx.done(&id, &format!("deleted marker {id}"), || json!({ "id": id }));
            Ok(())
        }
        Cmd::Selection => {
            let mut context = ctx.context.clone();
            context.selection = None;
            context.save()?;
            if let Ok(client) = ctx.client() {
                let _ = client.select(&Selection::set(0, ""));
            }
            ctx.done("", "selection cleared", || json!({ "selection": null }));
            Ok(())
        }
    }
}

fn scope(ctx: &Ctx, scope: u32) -> Result<()> {
    let client = ctx.client()?;
    check(client.clear(scope)?, "clear")?;
    if scope == clear_scope::ALL || scope == clear_scope::MARKERS {
        for i in 0..32 {
            let _ = client.marker_delete(&format!("target_marker_{i}"));
        }
    }
    let name = clear_scope::name(scope);
    ctx.done(
        name,
        &format!("cleared {name}"),
        || json!({ "scope": name }),
    );
    Ok(())
}

fn confirm(ctx: &Ctx, question: &str) -> Result<bool> {
    if ctx.json || !std::io::stdin().is_terminal() {
        return Ok(true);
    }
    let instance = ctx.target().map(|t| t.name.clone()).unwrap_or_default();
    print!("gearbox [{instance}]: {question} [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}
