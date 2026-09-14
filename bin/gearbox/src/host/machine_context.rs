//! Pointer interaction with terrain-mounted machine labels.

use super::ground_card::GroundCard;
use crate::{
    attach::LocalAttachments,
    viewer::machine_context::{CouplingAction, MachineCards, MachineHover},
};
use bevy::prelude::*;
use mara::ui::modules::bevy::ChaseCamera;

#[derive(Default)]
pub(super) struct MachineContext {
    root: Option<Entity>,
    last_hover: f64,
    card: Option<GroundCard>,
    hit: Option<[egui::Pos2; 4]>,
    buttons: Vec<(egui::Rect, CouplingAction)>,
    pressed_action: Option<CouplingAction>,
    captured_drag: bool,
    notice: Option<(bool, String, f64)>,
}

fn contains(quad: &[egui::Pos2; 4], p: egui::Pos2) -> bool {
    let mut positive = false;
    let mut negative = false;
    for i in 0..4 {
        let edge = quad[(i + 1) % 4] - quad[i];
        let delta = p - quad[i];
        let cross = edge.x * delta.y - edge.y * delta.x;
        positive |= cross > 0.001;
        negative |= cross < -0.001;
    }
    positive != negative
}

fn coupling_button(
    ctx: &egui::Context,
    viewport: egui::Rect,
    rect: egui::Rect,
    endpoints: [egui::Pos2; 2],
    action: &CouplingAction,
) {
    let painter = ctx
        .layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("hitch_point_buttons"),
        ))
        .with_clip_rect(viewport);
    let hovered = ctx.pointer_hover_pos().is_some_and(|p| rect.contains(p));
    let color = if action.enabled {
        egui::Color32::from_rgb(86, 218, 184)
    } else {
        egui::Color32::from_rgb(158, 166, 173)
    };
    let stroke = egui::Stroke::new(1.5, color);
    for point in endpoints {
        painter.line_segment([rect.center_bottom(), point], egui::Stroke::new(1.0, color));
        painter.circle_filled(point, 3.0, color);
        painter.circle_stroke(point, 6.0, stroke);
    }
    painter.rect_filled(rect, 8.0, egui::Color32::from_rgb(22, 28, 30));
    painter.rect_stroke(
        rect,
        8.0,
        egui::Stroke::new(if hovered { 2.0 } else { 1.0 }, color),
        egui::StrokeKind::Inside,
    );
    let center = egui::pos2(rect.left() + 18.0, rect.center().y);
    for offset in [-4.0, 4.0] {
        let link =
            egui::Rect::from_center_size(center + egui::vec2(offset, 0.0), egui::vec2(11.0, 8.0));
        painter.rect_stroke(link, 4.0, stroke, egui::StrokeKind::Middle);
    }
    if matches!(
        action.action,
        crate::attach::LocalAttachmentAction::Disconnect { .. }
    ) {
        painter.line_segment(
            [
                center + egui::vec2(-5.0, 7.0),
                center + egui::vec2(5.0, -7.0),
            ],
            stroke,
        );
    } else {
        painter.line_segment(
            [center - egui::vec2(3.0, 0.0), center + egui::vec2(3.0, 0.0)],
            stroke,
        );
    }
    painter.text(
        egui::pos2(rect.left() + 35.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &action.label,
        egui::FontId::proportional(13.0),
        color,
    );
    if hovered {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
}

impl MachineContext {
    pub fn interact(&mut self, ctx: &egui::Context, world: &mut World) {
        let (pointer, pressed, released, down) = ctx.input(|i| {
            (
                i.pointer.hover_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.any_down(),
            )
        });
        let pointer = pointer.filter(|p| {
            ctx.layer_id_at(*p)
                .is_none_or(|layer| layer.order == egui::Order::Background)
        });
        let button = pointer.and_then(|p| {
            self.buttons
                .iter()
                .find(|(rect, _)| rect.contains(p))
                .map(|(_, a)| a)
        });
        let over = button.is_some()
            || pointer.is_some_and(|p| self.hit.as_ref().is_some_and(|quad| contains(quad, p)));
        if pressed && over {
            self.pressed_action = button.cloned();
            self.captured_drag = true;
        }
        world.resource_mut::<MachineHover>().captures_pointer = over || self.captured_drag;
        if released {
            if let Some(action) = self.pressed_action.take()
                && let Some(current) = button.filter(|a| a.action == action.action)
            {
                let mut local = world.resource_mut::<LocalAttachments>();
                if current.enabled {
                    local.pending.push(action.action.clone());
                } else {
                    local.feedback = Some((false, current.detail.clone()));
                }
            }
        }
        if !down {
            self.captured_drag = false;
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, viewport: egui::Rect, world: &mut World) {
        let now = ctx.input(|i| i.time);
        if let Some((ok, message)) = world.resource_mut::<LocalAttachments>().feedback.take() {
            self.notice = Some((ok, message, now));
        }
        let hover = world.resource::<MachineHover>();
        let cards = &world.resource::<MachineCards>().0;
        let hit = hover
            .hit
            .filter(|root| cards.iter().any(|c| c.root == *root));
        if !hover.captures_pointer
            && let Some(root) = hit
        {
            self.root = Some(root);
            self.last_hover = now;
        } else if hover.captures_pointer {
            self.last_hover = now;
        }
        if now - self.last_hover > 0.7 {
            self.root = None;
        }
        let selected = world.resource::<crate::viewer::systems::Selection>().0;
        let card = cards
            .iter()
            .find(|c| Some(c.root) == self.root.or(selected))
            .cloned();
        world.resource_mut::<MachineHover>().retained = card.as_ref().map(|c| c.root);
        self.hit = None;
        self.buttons.clear();
        if let Some(card) = card {
            let plate = self.card.get_or_insert_with(|| GroundCard::new(world));
            let corners = plate.show(ctx, world, &card, &card.ns, selected == Some(card.root));
            let mut camera =
                world.query_filtered::<(&Camera, &GlobalTransform), With<ChaseCamera>>();
            if let Ok((camera, gt)) = camera.single(world)
                && let Some(size) = camera.logical_viewport_size()
            {
                let project = |point| {
                    camera.world_to_viewport(gt, point).ok().map(|p| {
                        viewport.min
                            + egui::vec2(
                                p.x / size.x * viewport.width(),
                                p.y / size.y * viewport.height(),
                            )
                    })
                };
                let points: Option<Vec<_>> = corners.into_iter().map(project).collect();
                if let Some(points) = points {
                    self.hit = Some([points[0], points[1], points[2], points[3]]);
                }
            }
        } else if let Some(card) = &self.card {
            card.hide(world);
        }
        self.show_couplings(ctx, viewport, world);
        if let Some((ok, text, at)) = &self.notice
            && now - at < 5.0
        {
            egui::Area::new(egui::Id::new("coupling_notice"))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(
                    viewport.center().x - 180.0,
                    viewport.bottom() - 64.0,
                ))
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(text).color(if *ok {
                                egui::Color32::from_rgb(86, 218, 184)
                            } else {
                                egui::Color32::from_rgb(245, 179, 111)
                            }));
                        });
                });
        }
    }

    fn show_couplings(&mut self, ctx: &egui::Context, viewport: egui::Rect, world: &mut World) {
        let Some(pointer) = ctx.pointer_hover_pos().filter(|p| {
            viewport.contains(*p)
                && ctx
                    .layer_id_at(*p)
                    .is_none_or(|layer| layer.order == egui::Order::Background)
        }) else {
            return;
        };
        let mut camera = world.query_filtered::<(&Camera, &GlobalTransform), With<ChaseCamera>>();
        let Ok((camera, gt)) = camera.single(world) else {
            return;
        };
        let Some(size) = camera.logical_viewport_size() else {
            return;
        };
        let project = |point| {
            camera.world_to_viewport(gt, point).ok().map(|p| {
                viewport.min
                    + egui::vec2(
                        p.x / size.x * viewport.width(),
                        p.y / size.y * viewport.height(),
                    )
            })
        };
        for card in &world.resource::<MachineCards>().0 {
            for action in &card.actions {
                if self.buttons.iter().any(|(_, a)| a.action == action.action) {
                    continue;
                }
                let [Some(a), Some(b)] = action.endpoints.map(project) else {
                    continue;
                };
                let anchor = a.lerp(b, 0.5);
                if !viewport.contains(anchor) {
                    continue;
                }
                let mut rect = egui::Rect::from_center_size(
                    anchor - egui::vec2(0.0, 36.0),
                    egui::vec2(116.0, 30.0),
                );
                while self.buttons.iter().any(|(other, _)| other.intersects(rect)) {
                    rect = rect.translate(egui::vec2(0.0, -34.0));
                }
                if pointer
                    .distance(a)
                    .min(pointer.distance(b))
                    .min(pointer.distance(anchor))
                    > 64.0
                    && !rect.expand(12.0).contains(pointer)
                {
                    continue;
                }
                coupling_button(ctx, viewport, rect, [a, b], action);
                self.buttons.push((rect, action.clone()));
            }
        }
    }
}
