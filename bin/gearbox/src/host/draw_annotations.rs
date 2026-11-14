use std::sync::Arc;

use bevy::prelude::World;
use mara::host::MaraHostCtx;
use mara::ui::mara_core::vocab::Id as MaraId;

use crate::draw::{AnnotationMeta, DrawLabel, DrawLabels, DrawPanel, DrawPanels, visual};
use crate::viewer::tf_overlay::TfLabels;
use super::{draw_labels, draw_overlay};

#[derive(Clone)]
struct Frame {
    pass: u64,
    viewport: egui::Rect,
    labels: Vec<DrawLabel>,
    panels: Vec<DrawPanel>,
    reserved: Vec<egui::Rect>,
}

fn frame_key(ctx: &egui::Context) -> egui::Id {
    egui::Id::new(("gearbox_draw_annotations", ctx.viewport_id()))
}

fn chrome_rects(ctx: &egui::Context, viewport: egui::Rect) -> Vec<egui::Rect> {
    let host = MaraHostCtx::ui_only(ctx, None);
    let shell = mara::ui::mara_core::window_chrome::__internal_window_chrome_regions(&host.seam()).exclusion_rects;
    let actions = [super::ACTION_PLAY, super::ACTION_CLEAR].into_iter()
        .filter_map(|id| ctx.read_response(MaraId::new(("mara_ribbon_btn_hit", id)).into()))
        .map(|response| ctx.layer_transform_to_global(response.layer_id).unwrap_or(egui::emath::TSTransform::IDENTITY) * response.rect);
    let mut rects = Vec::new();
    for rect in shell.into_iter().map(egui::Rect::from).chain(actions).filter(|rect| rect.is_finite()) {
        let rect = rect.intersect(viewport);
        if rect.is_positive() && !rects.contains(&rect) { rects.push(rect); }
    }
    rects
}

fn paint_pending(ctx: &egui::Context) {
    let key = frame_key(ctx);
    let Some(frame) = ctx.data_mut(|data| data.remove_temp::<Option<Frame>>(key)).flatten() else { return; };
    if frame.pass != ctx.cumulative_pass_nr() { return; }
    let mut reserved = frame.reserved;
    reserved.extend(chrome_rects(ctx, frame.viewport));
    let mut prepared = draw_labels::prepare_reserved(ctx, frame.viewport, &frame.labels, &reserved);
    let panels = draw_overlay::prepare_panels(ctx, frame.viewport, &frame.panels, &mut prepared.occupied);
    draw_labels::place_remaining(frame.viewport, &mut prepared);
    draw_overlay::paint_panels(ctx, frame.viewport, &panels, &prepared.occupied);
    draw_labels::paint_prepared(ctx, frame.viewport, prepared);
}

pub(super) fn queue(ctx: &egui::Context, viewport: egui::Rect, world: &World, reserved: &[egui::Rect]) {
    let installed = egui::Id::new("gearbox_draw_annotations_installed");
    let first = ctx.data_mut(|data| {
        let first = !data.get_temp::<bool>(installed).unwrap_or(false);
        data.insert_temp(installed, true);
        first
    });
    if first { ctx.on_end_pass("gearbox_draw_annotations", Arc::new(|ui| paint_pending(ui.ctx()))); }
    let mut labels = world.get_resource::<DrawLabels>().map(|labels| labels.0.clone()).unwrap_or_default();
    if let Some(frames) = world.get_resource::<TfLabels>() {
        labels.extend(frames.0.iter().map(|frame| DrawLabel {
            text: frame.name.clone(), at: frame.at, color: visual::INK, background: [0; 4], accent: visual::ACCENT,
            annotation: AnnotationMeta { icon: "frame".into(), ..Default::default() },
            size_px: 14.0, centred: false, opacity: frame.opacity,
        }));
    }
    let panels = world.get_resource::<DrawPanels>().map(|panels| panels.0.clone()).unwrap_or_default();
    let frame = Frame { pass: ctx.cumulative_pass_nr(), viewport, labels, panels, reserved: reserved.to_vec() };
    let key = frame_key(ctx);
    ctx.data_mut(|data| data.insert_temp(key, Some(frame)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::PanelChart;
    use mara::ui::mara_core::{RibbonDrag, RibbonOpen, RibbonPlacement, ShellBar};

    fn panel_world(at: [f32; 2]) -> World {
        let mut world = World::default();
        world.insert_resource(DrawPanels(vec![DrawPanel {
            key: ("chrome".into(), "panel".into()), at: Some(at), opacity: 1.0,
            chart: PanelChart { kind: gearbox_api::panel_kind::CHIP, text: "Inspection".into(),
                color: [200, 100, 150, 255], ..Default::default() },
        }]));
        world
    }

    fn panel_bounds(output: &egui::FullOutput) -> Vec<egui::Rect> {
        output.shapes.iter().filter_map(|shape| match &shape.shape {
            egui::Shape::Mesh(mesh) if mesh.vertices.len() == 7 && mesh.indices.len() == 18 =>
                Some(egui::Rect::from_points(&mesh.vertices.iter().map(|v| v.pos).collect::<Vec<_>>())),
            _ => None,
        }).collect()
    }

    #[test]
    fn late_native_shell_buttons_are_avoided_on_first_pass_and_resize() {
        let ctx = egui::Context::default();
        let world = panel_world([0.95, 0.16]);
        let mut bar = ShellBar { app_menu: false, ..Default::default() };
        let (mut open, mut placement, mut drag) = (RibbonOpen::default(), RibbonPlacement::default(), RibbonDrag::default());
        for size in [egui::vec2(640.0, 400.0), egui::vec2(900.0, 500.0), egui::vec2(480.0, 700.0), egui::vec2(640.0, 400.0)] {
            let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            let mut buttons = Vec::new();
            let mut expected = 0;
            let output = crate::host::run_ui_clean(&ctx, egui::RawInput { screen_rect: Some(viewport), ..Default::default() }, |ui| {
                let host = MaraHostCtx::mara_window(ui.ctx(), None);
                host.apply_default_theme();
                host.publish_full_shelf_layout();
                queue(ui.ctx(), viewport, &world, &[]);
                host.show_shell_bar(&mut bar, &mut open, &mut placement, &mut drag);
                buttons = chrome_rects(ui.ctx(), viewport);
                expected = if mara::ui::mara_core::style::screen_class() == mara::ui::mara_core::style::Breakpoint::Phone { 0 } else { 2 };
            });
            assert_eq!(buttons.len(), expected, "{size:?}");
            let panels = panel_bounds(&output);
            assert_eq!(panels.len(), 1);
            assert!(viewport.contains_rect(panels[0]));
            assert!(buttons.iter().all(|button| !panels[0].intersects(*button)), "{size:?}: {:?} / {buttons:?}", panels[0]);
        }
    }

    #[test]
    fn annotation_snapshots_are_latest_only_and_consumed_each_pass() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 600.0));
        let world = panel_world([0.5, 0.5]);
        let output = crate::host::run_ui_clean(&ctx, egui::RawInput { screen_rect: Some(viewport), ..Default::default() }, |ui| {
            queue(ui.ctx(), viewport, &panel_world([0.2, 0.8]), &[]);
            queue(ui.ctx(), viewport, &world, &[]);
        });
        let panels = panel_bounds(&output);
        assert_eq!(panels.len(), 1);
        assert!((panels[0].center().x - 450.0).abs() < 0.001);
        let absent = crate::host::run_ui_clean(&ctx, egui::RawInput { screen_rect: Some(viewport), ..Default::default() }, |_| {});
        assert!(panel_bounds(&absent).is_empty());
        let cleared = crate::host::run_ui_clean(&ctx, egui::RawInput { screen_rect: Some(viewport), ..Default::default() }, |ui| {
            queue(ui.ctx(), viewport, &World::default(), &[]);
        });
        assert!(panel_bounds(&cleared).is_empty());
    }

    #[test]
    fn chrome_bounds_follow_layer_transform_and_viewport_clip() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(130.0, 120.0));
        let _ = crate::host::run_ui_clean(&ctx, egui::RawInput { screen_rect: Some(viewport), ..Default::default() }, |ui| {
            let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(34.0, 34.0));
            ui.interact(rect, MaraId::new(("mara_ribbon_btn_hit", super::super::ACTION_PLAY)).into(), egui::Sense::click());
            ui.ctx().set_transform_layer(ui.layer_id(), egui::emath::TSTransform::from_translation(egui::vec2(100.0, 50.0)));
            assert_eq!(chrome_rects(ui.ctx(), viewport), vec![egui::Rect::from_min_max(egui::pos2(110.0, 70.0), egui::pos2(130.0, 104.0))]);
        });
    }
}
