//! View: overlays, the TF tree, render knobs and saved camera views.

use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::container::{Tab, TabContainer};
use mara_core::pod::{Pod, PodResponse};
use mara_core::vocab::Id as MaraId;
use std::collections::HashMap;

use super::{PaneCtx, button_clicked, cid, pid, pod_response, select_list_clicked, set_toggle};
use crate::host::PANE_VIEW as P;
use crate::viewer::commands::HostCommand;
use crate::viewer::overlays::DisplayToggles;
use crate::viewer::state::CameraBookmarks;

pub fn tab(world: &World, ctx: &PaneCtx) -> Tab {
    let accent = ctx.accent;
    let toggles = world.resource::<DisplayToggles>().clone();

    let overlays_pod = pid(P, "overlays", 0);
    ctx.sync_toggles(
        overlays_pod,
        &[
            toggles.show_world_grid,
            toggles.show_world_axes,
            toggles.show_physics,
            toggles.show_colliders,
            toggles.unlimited_zoom,
            toggles.follow_from_behind,
        ],
    );
    let overlays_pod_built = Pod::new(overlays_pod)
        .with_toggle_initial("Ground grid", accent, toggles.show_world_grid)
        .with_toggle_initial("World axes", accent, toggles.show_world_axes)
        .with_toggle_initial("Physics", accent, toggles.show_physics)
        .with_toggle_initial("Colliders", accent, toggles.show_colliders)
        .with_toggle_initial("Unlimited zoom", accent, toggles.unlimited_zoom)
        .with_toggle_initial("Follow from behind", accent, toggles.follow_from_behind);

    let tf_pod = pid(P, "tf", 0);
    ctx.sync_toggles(
        tf_pod,
        &[toggles.show_tf_frames, toggles.show_tf_names, toggles.show_tf_links, toggles.tf_wheels_only],
    );
    let tf_pod_built = Pod::new(tf_pod)
        .with_toggle_initial("Frames", accent, toggles.show_tf_frames)
        .with_toggle_initial("Names", accent, toggles.show_tf_names)
        .with_toggle_initial("Parent links", accent, toggles.show_tf_links)
        .with_toggle_initial("Wheels only", accent, toggles.tf_wheels_only)
        .with_readout("wheel axes", "Steering + spin; knuckles steer only");

    let render_pod = pid(P, "render", 0);
    let wireframe_label = if world.resource::<crate::viewer::overlays::WireframeAvailable>().0 {
        "Wireframe"
    } else {
        "Wireframe (GPU unavailable)"
    };
    ctx.sync_toggles(render_pod, &[toggles.wireframe]);
    let render_pod_built = Pod::new(render_pod)
        .with_toggle_initial(wireframe_label, accent, toggles.wireframe)
        .with_slider(
            "Light",
            (toggles.light_intensity_scale as f64).clamp(0.0, 4.0),
            0.0..=4.0,
            2,
            "x",
            accent,
        );

    // Saved camera views: save, clear, and one row per view to recall it.
    let bookmarks = world.resource::<CameraBookmarks>();
    let mut camera_pods = vec![
        Pod::new(pid(P, "cameras", 0))
            .with_button("Save view", accent)
            .with_button("Clear", accent),
    ];
    if !bookmarks.items.is_empty() {
        let rows: Vec<String> = bookmarks.items.iter().map(|b| b.name.clone()).collect();
        let trailing: Vec<String> =
            bookmarks.items.iter().map(|b| format!("{:.1} m", b.view.distance_m)).collect();
        camera_pods.push(
            Pod::new(pid(P, "cameras", 1)).with_select_list(rows, Some(trailing), accent),
        );
    }

    // Same order as the flat list was, so `apply`'s pod indices hold.
    Tab::new(cid(P, "view"), "View", "camera").containers(vec![
        TabContainer::new(cid(P, "overlays"), "Overlays", "eye", vec![overlays_pod_built]),
        TabContainer::new(cid(P, "tf"), "TF frames", "cube", vec![tf_pod_built]),
        TabContainer::new(cid(P, "render"), "Render", "image", vec![render_pod_built]),
        TabContainer::new(cid(P, "cameras"), "Saved views", "bookmark", camera_pods),
    ])
}

pub fn apply(responses: &HashMap<MaraId, Vec<PodResponse>>, world: &mut World, ctx: &PaneCtx) {
    let view_id = cid(P, "view");
    if button_clicked(responses, view_id, 3, 0) {
        ctx.send(HostCommand::SaveBookmark);
    }
    if button_clicked(responses, view_id, 3, 1) {
        ctx.send(HostCommand::ClearBookmarks);
    }
    if let Some(index) = select_list_clicked(responses, view_id, 4) {
        ctx.send(HostCommand::RecallBookmark(index));
    }
    let mut toggles = world.resource_mut::<DisplayToggles>();
    if let Some(resp) = pod_response(responses, view_id, 0) {
        set_toggle(&mut toggles.show_world_grid, resp, 0);
        set_toggle(&mut toggles.show_world_axes, resp, 1);
        set_toggle(&mut toggles.show_physics, resp, 2);
        set_toggle(&mut toggles.show_colliders, resp, 3);
        set_toggle(&mut toggles.unlimited_zoom, resp, 4);
        set_toggle(&mut toggles.follow_from_behind, resp, 5);
    }
    if let Some(resp) = pod_response(responses, view_id, 1) {
        set_toggle(&mut toggles.show_tf_frames, resp, 0);
        set_toggle(&mut toggles.show_tf_names, resp, 1);
        set_toggle(&mut toggles.show_tf_links, resp, 2);
        set_toggle(&mut toggles.tf_wheels_only, resp, 3);
    }
    if let Some(resp) = pod_response(responses, view_id, 2) {
        set_toggle(&mut toggles.wireframe, resp, 0);
        if let Some(slider) = resp.sliders.first()
            && slider.changed
        {
            toggles.light_intensity_scale = slider.value as f32;
        }
    }
}
