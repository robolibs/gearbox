//! `gearbox select`: CLI context plus the viewer highlight, kept in step
//! through `/gearbox/select`.

use clap::{Args as ClapArgs, Subcommand};
use gearbox_api::{Selection, object_kind};
use serde_json::json;

use crate::ctx::Ctx;
use crate::error::{CliError, Result};
use crate::paths::SelectionCtx;

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Current selection
    Show,
    /// Select a machine by namespace
    Machine { ns: String },
    /// Select a loaded object by id
    Object { id: String },
    /// Select a marker by id
    Marker { id: String },
    /// Select the next object in `scene list` order
    Next,
    /// Select the previous object in `scene list` order
    Prev,
    /// Clear the selection
    None,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::Show => {
            let live = ctx
                .client()
                .ok()
                .and_then(|c| c.select(&Selection::get()).ok());
            let ctx_sel = ctx.context.selection.clone();
            ctx.emit(
                || {
                    json!({
                        "context": ctx_sel.as_ref().map(|s| json!({ "kind": s.kind, "id": s.id })),
                        "viewer": live.as_ref().map(|s| json!({ "kind": object_kind::name(s.kind), "id": s.id() })),
                    })
                },
                || {
                    match &ctx_sel {
                        Some(s) => println!("context  {} {}", s.kind, s.id),
                        None => println!("context  none"),
                    }
                    match &live {
                        Some(s) if !s.id().is_empty() => {
                            println!("viewer   {} {}", object_kind::name(s.kind), s.id())
                        }
                        Some(_) => println!("viewer   none"),
                        None => println!("viewer   unreachable"),
                    }
                },
            );
            Ok(())
        }
        Cmd::Machine { ns } => set(ctx, object_kind::MACHINE, &ns),
        Cmd::Object { id } => set(ctx, object_kind::PROP, &id),
        Cmd::Marker { id } => set(ctx, object_kind::MARKER, &id),
        Cmd::Next => cycle(ctx, 1),
        Cmd::Prev => cycle(ctx, -1),
        Cmd::None => {
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

fn set(ctx: &Ctx, kind: u32, id: &str) -> Result<()> {
    let client = ctx.client()?;
    let exists = client.list(object_kind::ANY)?.iter().any(|o| {
        o.props().get("id").as_deref() == Some(id)
            || o.props().get("namespace").as_deref() == Some(id)
    });
    if !exists {
        return Err(CliError::new(1, format!("no `{id}` in the scene")));
    }
    client.select(&Selection::set(kind, id))?;
    let mut context = ctx.context.clone();
    context.selection = Some(SelectionCtx {
        kind: object_kind::name(kind).to_string(),
        id: id.to_string(),
    });
    context.save()?;
    ctx.done(
        id,
        &format!("selected {} {id}", object_kind::name(kind)),
        || json!({ "kind": object_kind::name(kind), "id": id }),
    );
    Ok(())
}

fn cycle(ctx: &Ctx, step: i64) -> Result<()> {
    let client = ctx.client()?;
    let objects = client.list(object_kind::ANY)?;
    if objects.is_empty() {
        return Err(CliError::new(1, "scene is empty"));
    }
    let ids: Vec<(u32, String)> = objects
        .iter()
        .map(|o| (o.kind, o.props().get("id").unwrap_or_default()))
        .collect();
    let current = ctx
        .context
        .selection
        .as_ref()
        .and_then(|s| ids.iter().position(|(_, id)| *id == s.id));
    let next = match current {
        Some(i) => (i as i64 + step).rem_euclid(ids.len() as i64) as usize,
        None if step > 0 => 0,
        None => ids.len() - 1,
    };
    let (kind, id) = ids[next].clone();
    set(ctx, kind, &id)
}
