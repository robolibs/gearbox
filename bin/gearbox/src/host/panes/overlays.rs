//! Overlay toggles and render knobs, written straight to `DisplayToggles`.

use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;

use super::{PaneCtx, cid, pid, pod_response, set_toggle};
use crate::host::PANE_OVERLAYS as P;
use crate::viewer::overlays::DisplayToggles;
use crate::viewer::state::LoaderTuning;

pub fn show(body: &mut PaneBody<'_, '_>, world: &mut World, ctx: &PaneCtx) {
    let accent = ctx.accent;
    let toggles = world.resource::<DisplayToggles>().clone();
    let curve_radius = world.resource::<LoaderTuning>().curves.default_radius;

    let toggles_id = cid(P, "toggles");
    let toggles_pod = pid(P, "toggles", 0);
    ctx.sync_toggles(
        toggles_pod,
        &[
            toggles.show_world_grid,
            toggles.show_world_axes,
            toggles.show_prim_markers,
            toggles.show_skeleton,
            toggles.show_physics,
            toggles.show_colliders,
        ],
    );
    body.add_normal(
        toggles_id,
        "World overlays",
        "color",
        vec![
            Pod::new(toggles_pod)
                .with_toggle_initial("World grid", accent, toggles.show_world_grid)
                .with_toggle_initial("World axes", accent, toggles.show_world_axes)
                .with_toggle_initial("Prim markers", accent, toggles.show_prim_markers)
                .with_toggle_initial("Skeleton", accent, toggles.show_skeleton)
                .with_toggle_initial("Physics overlay", accent, toggles.show_physics)
                .with_toggle_initial("Colliders", accent, toggles.show_colliders),
        ],
    );
    let tf_id = cid(P, "tf");
    let tf_pod = pid(P, "tf", 0);
    ctx.sync_toggles(
        tf_pod,
        &[
            toggles.show_tf_frames,
            toggles.show_tf_names,
            toggles.show_tf_links,
        ],
    );
    body.add_normal(
        tf_id,
        "TF tree",
        "branch",
        vec![
            Pod::new(tf_pod)
                .with_toggle_initial("Frames", accent, toggles.show_tf_frames)
                .with_toggle_initial("Names", accent, toggles.show_tf_names)
                .with_toggle_initial("Parent links", accent, toggles.show_tf_links),
        ],
    );
    let render_id = cid(P, "render");
    let render_pod = pid(P, "render", 0);
    ctx.sync_toggles(render_pod, &[toggles.wireframe]);
    body.add_normal(
        render_id,
        "Render",
        "options",
        vec![
            Pod::new(render_pod)
                .with_toggle_initial("Wireframe", accent, toggles.wireframe)
                .with_slider(
                    "Light intensity",
                    (toggles.light_intensity_scale as f64).clamp(0.0, 4.0),
                    0.0..=4.0,
                    2,
                    "x",
                    accent,
                )
                .with_slider(
                    "Curve radius",
                    (curve_radius as f64).clamp(0.001, 0.25),
                    0.001..=0.25,
                    3,
                    " m",
                    accent,
                ),
        ],
    );

    let responses = body.render();
    let mut toggles = world.resource_mut::<DisplayToggles>();
    if let Some(resp) = pod_response(&responses, toggles_id, 0) {
        set_toggle(&mut toggles.show_world_grid, resp, 0);
        set_toggle(&mut toggles.show_world_axes, resp, 1);
        set_toggle(&mut toggles.show_prim_markers, resp, 2);
        set_toggle(&mut toggles.show_skeleton, resp, 3);
        set_toggle(&mut toggles.show_physics, resp, 4);
        set_toggle(&mut toggles.show_colliders, resp, 5);
    }
    if let Some(resp) = pod_response(&responses, tf_id, 0) {
        set_toggle(&mut toggles.show_tf_frames, resp, 0);
        set_toggle(&mut toggles.show_tf_names, resp, 1);
        set_toggle(&mut toggles.show_tf_links, resp, 2);
    }
    if let Some(resp) = pod_response(&responses, render_id, 0) {
        set_toggle(&mut toggles.wireframe, resp, 0);
        if let Some(slider) = resp.sliders.first()
            && slider.changed
        {
            toggles.light_intensity_scale = slider.value as f32;
        }
        if let Some(slider) = resp.sliders.get(1)
            && slider.changed
        {
            world.resource_mut::<LoaderTuning>().curves.default_radius = slider.value as f32;
        }
    }
}
