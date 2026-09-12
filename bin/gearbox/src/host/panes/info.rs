//! Stage info: composition, lights and render counts, physics counts of
//! the active stage.

use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;

use super::{PaneCtx, cid, nonempty_or, pid, short_path};
use crate::host::PANE_INFO as P;
use crate::viewer::state::StageInfo;

pub fn show(body: &mut PaneBody<'_, '_>, world: &mut World, ctx: &PaneCtx) {
    let accent = ctx.accent;
    let info = world.resource::<StageInfo>();
    body.add_normal(
        cid(P, "stage"),
        "Composition",
        "document",
        vec![
            Pod::new(pid(P, "stage", 0))
                .with_readout("path", short_path(nonempty_or(&info.path, "No active stage"), 34))
                .with_readout(
                    "default prim",
                    info.default_prim.as_deref().unwrap_or("None"),
                )
                .with_readout("layers", info.layer_count.to_string())
                .with_readout("variants", info.variant_count.to_string())
                .with_readout("custom attrs", info.custom_attr_prim_count.to_string()),
        ],
    );
    body.add_normal(
        cid(P, "render"),
        "Render / lights",
        "color",
        vec![
            Pod::new(pid(P, "render", 0))
                .with_badge_row(
                    "lights",
                    vec![
                        format!("dir {}", info.lights_directional),
                        format!("point {}", info.lights_point),
                        format!("spot {}", info.lights_spot),
                        format!("dome {}", info.lights_dome),
                    ],
                    accent,
                )
                .with_badge_row(
                    "render",
                    vec![
                        format!("settings {}", info.render_settings_count),
                        format!("products {}", info.render_product_count),
                        format!("vars {}", info.render_var_count),
                    ],
                    accent,
                ),
        ],
    );
    body.add_normal(
        cid(P, "physics"),
        "Physics",
        "box",
        vec![
            Pod::new(pid(P, "physics", 0))
                .with_readout("rigid bodies", info.rigid_body_count.to_string())
                .with_readout("physics scenes", info.physics_scene_count.to_string())
                .with_readout("joints", info.joint_count.to_string()),
        ],
    );
}
