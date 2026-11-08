//! The host's retained draws: entities by `ns` + `id`, chunk assembly,
//! lifetimes in sim seconds and recursive path clears. Plain data; the Bevy
//! plugin serves it and the viewer draws it.

use std::collections::{BTreeMap, HashMap};

use crate::wire::Props;
use crate::wire::draw::*;

/// `ns` and `id` of an entity.
pub type DrawKey = (String, String);

/// A live entity as the viewer reads it.
#[derive(Debug, Clone)]
pub struct DrawItem {
    pub entity: DrawEntity,
    /// Sim second it goes away at; `None` keeps it.
    pub expires_s: Option<f64>,
    /// Bumped on every change to the entity.
    pub revision: u64,
}

#[derive(Debug, Clone)]
struct Pending {
    seq: u32,
    entity: DrawEntity,
    started_s: f64,
}

/// Sim seconds an unfinished chunked send is kept.
const PENDING_TIMEOUT_S: f64 = 30.0;

#[derive(Debug, Default, Clone)]
pub struct DrawStore {
    items: BTreeMap<DrawKey, DrawItem>,
    pending: HashMap<DrawKey, Pending>,
    revision: u64,
}

/// The default colour of an entity: a stable pick from its path.
pub fn default_color(ns: &str, id: &str) -> u32 {
    const PALETTE: [[u8; 3]; 10] = [
        [31, 119, 180],
        [255, 127, 14],
        [44, 160, 44],
        [214, 39, 40],
        [148, 103, 189],
        [23, 190, 207],
        [227, 119, 194],
        [188, 189, 34],
        [140, 86, 75],
        [127, 127, 127],
    ];
    let mut hash: u32 = 0x811c_9dc5;
    for b in ns.bytes().chain([b'/']).chain(id.bytes()) {
        hash = (hash ^ b as u32).wrapping_mul(0x0100_0193);
    }
    let [r, g, b] = PALETTE[(hash % PALETTE.len() as u32) as usize];
    rgba(r, g, b, 255)
}

impl DrawStore {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Bumped on every change to any entity.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn get(&self, ns: &str, id: &str) -> Option<&DrawItem> {
        self.items.get(&(ns.trim_matches('/').to_string(), id.to_string()))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&DrawKey, &DrawItem)> {
        self.items.iter()
    }

    /// Apply one message at sim second `now_s`; answers how many entities it
    /// touched.
    pub fn apply(&mut self, msg: DrawEntity, now_s: f64) -> Result<usize, String> {
        msg.validate()?;
        let key = (msg.ns().trim_matches('/').to_string(), msg.id());
        match msg.op {
            draw_op::UPSERT => {
                self.pending.remove(&key);
                self.store(key, msg, now_s);
                Ok(1)
            }
            draw_op::APPEND => self.append(key, msg, now_s),
            draw_op::DELETE => {
                self.pending.remove(&key);
                Ok(self.items.remove(&key).map(|_| self.bump()).is_some() as usize)
            }
            draw_op::CLEAR => Ok(self.clear(&key.0)),
            draw_op::EXTEND => {
                self.extend(key, msg, now_s);
                Ok(1)
            }
            draw_op::UPDATE => self.update(&key, &msg, now_s),
            draw_op::PATCH => self.patch(&key, &msg, now_s),
            op => Err(format!("unknown draw op {op}")),
        }
    }

    /// Takes the header groups `msg` names onto the live entity.
    fn update(&mut self, key: &DrawKey, msg: &DrawEntity, now_s: f64) -> Result<usize, String> {
        let Some(item) = self.items.get_mut(key) else {
            return Err(format!("no draw {}/{} to update", key.0, key.1));
        };
        let props = msg.props();
        let groups = props.get("set").unwrap_or_default();
        let takes = |g: &str| groups.is_empty() || groups.split(',').any(|s| s.trim() == g);
        let e = &mut item.entity;
        if takes("pose") {
            (e.px, e.py, e.pz, e.qw, e.qx, e.qy, e.qz) = (msg.px, msg.py, msg.pz, msg.qw, msg.qx, msg.qy, msg.qz);
        }
        if takes("scale") {
            (e.sx, e.sy, e.sz) = (msg.sx, msg.sy, msg.sz);
        }
        if takes("color") {
            (e.color, e.color2) = (msg.color, msg.color2);
        }
        if takes("width") {
            e.width = msg.width;
        }
        if takes("style") {
            e.style = msg.style;
        }
        if takes("effects") {
            (e.glow, e.pulse_hz, e.flow_speed, e.dash, e.beam_height, e.fade, e.urgency) =
                (msg.glow, msg.pulse_hz, msg.flow_speed, msg.dash, msg.beam_height, msg.fade, msg.urgency);
        }
        if takes("flags") {
            e.flags = msg.flags;
        }
        if takes("colormap") {
            (e.colormap, e.vmin, e.vmax) = (msg.colormap, msg.vmin, msg.vmax);
        }
        if takes("lifetime") {
            e.lifetime_s = msg.lifetime_s;
        }
        let mut merged = e.props();
        for (k, v) in props.iter() {
            if !matches!(k.as_str(), "ns" | "id" | "set") {
                merged.set(&k, &v);
            }
        }
        e.props = merged.into_bytes();
        item.expires_s = (e.lifetime_s > 0.0).then(|| now_s + e.lifetime_s);
        self.revision += 1;
        item.revision = self.revision;
        Ok(1)
    }

    /// Puts `msg`'s items over the live entity's from `chunk_offset` on.
    fn patch(&mut self, key: &DrawKey, msg: &DrawEntity, now_s: f64) -> Result<usize, String> {
        let Some(item) = self.items.get_mut(key) else {
            return Err(format!("no draw {}/{} to patch", key.0, key.1));
        };
        if item.entity.kind != msg.kind && msg.kind != 0 {
            return Err(format!("a {} patch for a {}", draw_kind::name(msg.kind), draw_kind::name(item.entity.kind)));
        }
        if item.entity.kind == draw_kind::GRID {
            patch_grid(&mut item.entity, msg)?;
        } else {
            patch_items(&mut item.entity, msg)?;
        }
        if msg.lifetime_s > 0.0 {
            item.entity.lifetime_s = msg.lifetime_s;
        }
        item.expires_s = (item.entity.lifetime_s > 0.0).then(|| now_s + item.entity.lifetime_s);
        self.revision += 1;
        item.revision = self.revision;
        Ok(1)
    }

    /// Remove every entity at `prefix` or below; `""` removes all.
    pub fn clear(&mut self, prefix: &str) -> usize {
        self.pending.retain(|(ns, _), _| !ns_within(ns, prefix));
        let before = self.items.len();
        self.items.retain(|(ns, _), _| !ns_within(ns, prefix));
        let removed = before - self.items.len();
        if removed > 0 {
            self.bump();
        }
        removed
    }

    /// Drop entities whose lifetime ran out and stale chunked sends.
    pub fn expire(&mut self, now_s: f64) -> usize {
        self.pending.retain(|_, p| now_s - p.started_s < PENDING_TIMEOUT_S && now_s >= p.started_s);
        let before = self.items.len();
        self.items.retain(|_, item| item.expires_s.is_none_or(|t| now_s < t));
        let removed = before - self.items.len();
        if removed > 0 {
            self.bump();
        }
        removed
    }

    /// Entities as header-only copies with props `points` and `instances`.
    pub fn summaries(&self, prefix: &str) -> Vec<DrawEntity> {
        self.items
            .iter()
            .filter(|((ns, _), _)| ns_within(ns, prefix))
            .map(|(_, item)| {
                let e = &item.entity;
                let props = e
                    .props()
                    .with("points", &e.point_count().to_string())
                    .with("instances", &e.instance_count().to_string());
                DrawEntity {
                    points: Vec::new(),
                    colors: Vec::new(),
                    radii: Vec::new(),
                    offsets: Vec::new(),
                    instances: Vec::new(),
                    values: Vec::new(),
                    rings: Vec::new(),
                    qpoints: Vec::new(),
                    ranges: Vec::new(),
                    cells: Vec::new(),
                    labels: Vec::new(),
                    props: props.with("cells", &e.cell_count().to_string()).into_bytes(),
                    ..e.clone()
                }
            })
            .collect()
    }

    fn bump(&mut self) -> u64 {
        self.revision += 1;
        self.revision
    }

    fn store(&mut self, key: DrawKey, mut entity: DrawEntity, now_s: f64) {
        entity.normalize();
        entity.op = draw_op::UPSERT;
        entity.last = 1;
        entity.chunk_offset = 0;
        let expires_s = (entity.lifetime_s > 0.0).then(|| now_s + entity.lifetime_s);
        let revision = self.bump();
        self.items.insert(key, DrawItem { entity, expires_s, revision });
    }

    fn append(&mut self, key: DrawKey, msg: DrawEntity, now_s: f64) -> Result<usize, String> {
        if msg.chunk_offset == 0 {
            self.pending.insert(key.clone(), Pending { seq: msg.seq, entity: msg.clone(), started_s: now_s });
        } else {
            let Some(pending) = self.pending.get_mut(&key) else {
                return Err(format!("chunk at {} of seq {} has no start", msg.chunk_offset, msg.seq));
            };
            if pending.seq != msg.seq || pending.entity.item_count() != msg.chunk_offset as usize {
                let expected = pending.entity.item_count();
                self.pending.remove(&key);
                return Err(format!("chunk at {} of seq {} out of order (expected {expected})", msg.chunk_offset, msg.seq));
            }
            append_items(&mut pending.entity, &msg, false);
        }
        if msg.last != 0
            && let Some(done) = self.pending.remove(&key)
        {
            let whole = DrawEntity { op: draw_op::UPSERT, ..done.entity };
            whole.validate().map_err(|err| format!("assembled draw {}/{}: {err}", key.0, key.1))?;
            self.store(key, whole, now_s);
        }
        Ok(1)
    }

    fn extend(&mut self, key: DrawKey, msg: DrawEntity, now_s: f64) {
        let max = msg.props().get("max_items").or_else(|| msg.props().get("max_points")).and_then(|v| v.parse::<usize>().ok());
        let Some(item) = self.items.get_mut(&key).filter(|item| item.entity.kind == msg.kind) else {
            let mut entity = msg;
            if let Some(max) = max {
                trim_front(&mut entity, max);
            }
            self.store(key, entity, now_s);
            return;
        };
        let continue_strip = msg.offsets.is_empty();
        append_items(&mut item.entity, &msg, continue_strip);
        if let Some(max) = max {
            trim_front(&mut item.entity, max);
        }
        if msg.lifetime_s > 0.0 {
            item.entity.lifetime_s = msg.lifetime_s;
        }
        item.expires_s = (item.entity.lifetime_s > 0.0).then(|| now_s + item.entity.lifetime_s);
        self.revision += 1;
        item.revision = self.revision;
    }
}

/// Per-unit values of `src` after `dst`'s, filling the side that had a
/// single value with `fill`.
fn merge_units<T: Copy>(dst: &mut Vec<T>, dst_units: usize, src: &[T], src_units: usize, fill: T) {
    if dst.is_empty() && src.is_empty() {
        return;
    }
    if dst.len() != dst_units {
        let one = if dst.len() == 1 { dst[0] } else { fill };
        *dst = vec![one; dst_units];
    }
    if src.len() == src_units {
        dst.extend_from_slice(src);
    } else {
        let one = if src.len() == 1 { src[0] } else { fill };
        dst.extend(std::iter::repeat_n(one, src_units));
    }
}

/// `src`'s items after `dst`'s; strips of `src` start anew unless
/// `continue_strip`, which runs its first strip on from `dst`'s last.
/// Polygon chunks with props `cont` go on with `dst`'s last polygon.
fn append_items(dst: &mut DrawEntity, src: &DrawEntity, continue_strip: bool) {
    let polygon = dst.kind == draw_kind::POLYGON;
    let strips = (dst.kind == draw_kind::LINES && dst.sub != line_mode::LIST) || polygon;
    let cont = if polygon { src.continues() } else { None };
    let continue_strip = continue_strip || cont.as_deref() == Some("ring");
    let base = dst.point_count() as u32;
    let dst_strips = dst.strip_ranges();
    let mut starts: Vec<u32> = if strips { dst_strips.iter().map(|r| r.start as u32).collect() } else { Vec::new() };
    let (dst_units, src_units) = (dst.unit_count(), src.unit_count());
    let (dst_labels, src_labels) = (dst.label_list(), src.label_list());
    let dst_label_units = dst.label_unit_count();

    if polygon {
        if dst.rings.is_empty() && !dst_strips.is_empty() {
            dst.rings = vec![dst_strips.len() as u32];
        }
        let mut src_rings = if src.rings.is_empty() && src.point_count() > 0 { vec![src.strip_ranges().len() as u32] } else { src.rings.clone() };
        if let (Some(c), Some(first)) = (cont.as_deref(), src_rings.first().copied())
            && let Some(last) = dst.rings.last_mut()
        {
            *last += first - (c == "ring") as u32;
            src_rings.remove(0);
        }
        dst.rings.extend(src_rings);
    }
    merge_units(&mut dst.colors, dst_units, &src.colors, src_units, 0u32);
    merge_units(&mut dst.radii, dst_units, &src.radii, src_units, 0.0f32);
    if dst.kind == draw_kind::GRID {
        dst.cells.extend_from_slice(&src.cells);
        dst.values.extend_from_slice(&src.values);
    } else if !matches!(dst.kind, draw_kind::GHOST | draw_kind::PANEL) {
        merge_units(&mut dst.values, dst_units, &src.values, src_units, 0.0f32);
        merge_units(&mut dst.cells, dst_units, &src.cells, src_units, 0u8);
    } else {
        dst.values.extend_from_slice(&src.values);
    }
    dst.points.extend_from_slice(&src.points);
    dst.qpoints.extend_from_slice(&src.qpoints);
    dst.ranges.extend_from_slice(&src.ranges);
    dst.instances.extend_from_slice(&src.instances);

    if strips {
        let src_starts = src.strip_ranges().into_iter().map(|r| r.start as u32 + base);
        let skip = (continue_strip && base > 0) as usize;
        starts.extend(src_starts.skip(skip));
        dst.offsets = if starts.len() > 1 { starts } else { Vec::new() };
    }
    if polygon && dst.rings.len() == 1 {
        dst.rings.clear();
    }
    if !dst_labels.is_empty() || !src_labels.is_empty() {
        let mut labels = dst_labels;
        labels.resize(dst_label_units, String::new());
        labels.extend(src_labels);
        labels.resize(dst.label_unit_count(), String::new());
        dst.labels = labels.join("\n").into_bytes();
    }
}

/// `msg`'s items written over `e`'s from `msg.chunk_offset`, the entity
/// growing when they run past its end.
fn patch_items(e: &mut DrawEntity, msg: &DrawEntity) -> Result<(), String> {
    let at = msg.chunk_offset as usize;
    let n = e.item_count();
    if at > n {
        return Err(format!("a patch at {at} past the {n} items"));
    }
    let units = e.unit_count();
    let unit_at = if e.kind == draw_kind::ARROWS && e.instances.is_empty() { at / 2 } else { at };
    let msg_units = msg.unit_count();
    if e.instanced() {
        splice(&mut e.instances, &msg.instances, at * INSTANCE_FLOATS);
    } else {
        splice(&mut e.points, &msg.points, at * 3);
        splice(&mut e.qpoints, &msg.qpoints, at * 3);
        splice(&mut e.ranges, &msg.ranges, at);
    }
    let grown = e.unit_count().max(units);
    patch_units(&mut e.colors, units, grown, &msg.colors, unit_at, msg_units, 0);
    patch_units(&mut e.radii, units, grown, &msg.radii, unit_at, msg_units, 0.0);
    patch_units(&mut e.values, units, grown, &msg.values, unit_at, msg_units, 0.0);
    patch_units(&mut e.cells, units, grown, &msg.cells, unit_at, msg_units, 0);
    let labels = msg.label_list();
    if !labels.is_empty() {
        let mut all = e.label_list();
        all.resize(e.label_unit_count(), String::new());
        for (k, l) in labels.into_iter().enumerate() {
            if let Some(slot) = all.get_mut(unit_at + k) {
                *slot = l;
            }
        }
        e.labels = all.join("\n").into_bytes();
    }
    e.validate()
}

/// `src` over `dst` from `at`, `dst` growing to hold it.
fn splice<T: Copy>(dst: &mut Vec<T>, src: &[T], at: usize) {
    if src.is_empty() {
        return;
    }
    if dst.len() < at + src.len() {
        let fill = src[0];
        dst.resize(at + src.len(), fill);
    }
    dst[at..at + src.len()].copy_from_slice(src);
}

/// Per-unit values `src` over `dst` from unit `at`, spreading a single
/// value of either side over its units first.
fn patch_units<T: Copy>(dst: &mut Vec<T>, units: usize, grown: usize, src: &[T], at: usize, src_units: usize, fill: T) {
    if src.is_empty() {
        if dst.len() == units && grown > units {
            let last = dst.last().copied().unwrap_or(fill);
            dst.resize(grown, last);
        }
        return;
    }
    if dst.len() != units || units == 0 {
        let one = dst.first().copied().unwrap_or(fill);
        *dst = vec![one; units];
    }
    let spread: Vec<T> = if src.len() == src_units { src.to_vec() } else { vec![src[0]; src_units] };
    splice(dst, &spread, at);
    if dst.len() < grown {
        let last = *dst.last().unwrap();
        dst.resize(grown, last);
    }
}

/// The rectangles of a grid patch written into the live grid.
fn patch_grid(e: &mut DrawEntity, msg: &DrawEntity) -> Result<(), String> {
    let (cols, rows, ..) = e.grid_dims();
    let size = grid_encoding::size(e.sub);
    if msg.sub != e.sub {
        return Err("a grid patch must use the grid's encoding".into());
    }
    if e.cells.is_empty() {
        let blank = match (e.sub, e.prop_f32("unknown")) {
            (grid_encoding::F32, u) => u.unwrap_or(f32::NAN).to_le_bytes().to_vec(),
            (grid_encoding::U16, u) => (u.unwrap_or(65535.0) as u16).to_le_bytes().to_vec(),
            (_, u) => vec![u.unwrap_or(255.0) as u8],
        };
        e.cells = blank.iter().copied().cycle().take(cols * rows * size).collect();
    }
    let mut read = 0usize;
    for r in msg.offsets.chunks_exact(4) {
        let (x, y, w, h) = (r[0] as usize, r[1] as usize, r[2] as usize, r[3] as usize);
        if x + w > cols || y + h > rows {
            return Err(format!("patch {w}×{h} at {x},{y} runs off the {cols}×{rows} grid"));
        }
        for row in 0..h {
            let dst = ((y + row) * cols + x) * size;
            let src = (read + row * w) * size;
            e.cells[dst..dst + w * size].copy_from_slice(&msg.cells[src..src + w * size]);
            if !msg.values.is_empty() {
                if e.values.len() != cols * rows {
                    e.values = vec![0.0; cols * rows];
                }
                let d = (y + row) * cols + x;
                let s = read + row * w;
                e.values[d..d + w].copy_from_slice(&msg.values[s..s + w]);
            }
        }
        read += w * h;
    }
    Ok(())
}

/// Keep the last `max` items (whole arrows and triangles) of `e`.
fn trim_front(e: &mut DrawEntity, max: usize) {
    let instanced = e.instanced();
    let step = match e.kind {
        draw_kind::ARROWS if !instanced => 2,
        draw_kind::TRIANGLES => 3,
        draw_kind::LINES if e.sub == line_mode::LIST => 2,
        _ => 1,
    };
    let n = e.item_count();
    if n <= max.max(step) {
        return;
    }
    let drop = (n - max.max(step)).div_ceil(step) * step;
    let unit_drop = if step == 2 && e.kind == draw_kind::ARROWS { drop / 2 } else { drop };
    let labels = e.label_list();
    let strips_before = e.strip_ranges();
    if instanced {
        e.instances.drain(..drop * INSTANCE_FLOATS);
    } else if e.kind == draw_kind::PANEL {
        e.values.drain(..drop);
    } else {
        drop_front(&mut e.points, drop * 3);
        drop_front(&mut e.qpoints, drop * 3);
        drop_front(&mut e.ranges, drop);
    }
    if e.colors.len() > unit_drop {
        e.colors.drain(..unit_drop);
    }
    if e.radii.len() > unit_drop {
        e.radii.drain(..unit_drop);
    }
    if e.kind != draw_kind::PANEL && e.values.len() > unit_drop {
        e.values.drain(..unit_drop);
    }
    if e.cells.len() > unit_drop {
        e.cells.drain(..unit_drop);
    }
    if e.kind == draw_kind::LINES && e.sub != line_mode::LIST {
        let kept: Vec<usize> = (0..strips_before.len()).filter(|i| strips_before[*i].end > drop).collect();
        let starts: Vec<u32> = kept.iter().map(|i| strips_before[*i].start.saturating_sub(drop) as u32).collect();
        e.offsets = if starts.len() > 1 { starts } else { Vec::new() };
        if !labels.is_empty() {
            let kept_labels: Vec<String> = kept.iter().map(|i| labels.get(*i).cloned().unwrap_or_default()).collect();
            e.labels = kept_labels.join("\n").into_bytes();
        }
    } else if !labels.is_empty() {
        let label_drop = if e.kind == draw_kind::TRIANGLES { drop / 3 } else { unit_drop };
        e.labels = labels.into_iter().skip(label_drop).collect::<Vec<_>>().join("\n").into_bytes();
    }
}

fn drop_front<T>(v: &mut Vec<T>, n: usize) {
    if !v.is_empty() {
        v.drain(..n.min(v.len()));
    }
}

/// Props of a status answer: how many entities a message touched.
pub fn touched_props(count: usize) -> Props {
    Props::from_pairs(&[("touched", &count.to_string())])
}
