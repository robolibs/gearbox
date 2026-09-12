//! Recent gearbox and USD log events.

use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;

use super::{PaneCtx, cid, pid};
use crate::host::PANE_LOG as P;

pub fn show(body: &mut PaneBody<'_, '_>, ctx: &PaneCtx) {
    let lines = ctx.log.tail(40);
    if lines.is_empty() {
        body.add_normal(
            cid(P, "lines"),
            "Recent events",
            "document",
            vec![
                Pod::new(pid(P, "lines", 0))
                    .with_readout("status", "No gearbox/USD log events captured yet"),
            ],
        );
        return;
    }
    body.add_normal(
        cid(P, "lines"),
        "Recent events",
        "document",
        vec![
            Pod::new(pid(P, "lines", 0))
                .fill()
                .with_select_list(lines, None, ctx.accent),
        ],
    );
}
