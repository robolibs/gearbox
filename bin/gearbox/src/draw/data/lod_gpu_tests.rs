use std::collections::BTreeSet;

use bevy::math::{Mat4, UVec4, Vec3, Vec4};
use gearbox_api::draw_data::CpuBudget;
use wgpu::util::DeviceExt;

use super::super::{
    cloud::Geometry,
    style::{PageUniform, uniform_bytes},
    tests::{fixture, stored},
    tree::ROOT,
};

/// One page's LOD pass result: the selected count, the per-node (offset, start) table,
/// every ordinal the vertex stage would draw, and what the next frame starts from.
struct Run {
    selected: u32,
    accepted: u32,
    table: Vec<[u32; 2]>,
    ordinals: Vec<u32>,
    chosen: u32,
    /// Points drawn at each threshold bin.
    points: Vec<i32>,
    next: Frame,
}

/// What a view's LOD pass keeps between frames: node states and the chosen bin.
#[derive(Clone)]
struct Frame {
    states: Vec<u32>,
    chosen: u32,
}

fn first() -> Frame {
    Frame { states: vec![0; 64], chosen: 0 }
}

/// The view's error bin for 1.5 px, as the LOD pass bins thresholds.
const FINEST: u32 = 35;

fn source() -> String {
    let lod = include_str!("../draw_cloud_lod.wesl")
        .lines()
        .filter(|line| !line.starts_with("import "))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{}\n{lod}", super::definitions())
}

fn run(geometry: &Geometry, clip_from_world: Mat4, budget: u32, before: &Frame) -> Run {
    let engine = molla_compute::Engine::new_gpu().unwrap();
    let device = engine.device().unwrap();
    let queue = engine.queue().unwrap();
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production cloud lod"),
        source: wgpu::ShaderSource::Wgsl(source().into()),
    });
    let storage = |label, contents: &[u8], usage| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage })
    };
    let rw = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
    let mut view = Vec::new();
    view.extend_from_slice(bytemuck::cast_slice(&clip_from_world.to_cols_array()));
    view.extend_from_slice(bytemuck::cast_slice(&[800.0f32, 600.0, 1.5]));
    view.extend_from_slice(&budget.to_le_bytes());
    let view = storage("lod view", &view, wgpu::BufferUsages::UNIFORM);
    let mut bins = vec![0u8; 656];
    bins[644..648].copy_from_slice(&before.chosen.to_le_bytes());
    let bins = storage("lod bins", &bins, rw);
    let page = PageUniform {
        world_from_local: Mat4::IDENTITY,
        clip_rotation: Mat4::IDENTITY,
        clip_min: Vec4::ZERO,
        clip_max: Vec4::ZERO,
        age: Default::default(),
        coverage: Vec4::ZERO,
        attributes: UVec4::ZERO,
        lod: Vec4::new(0.0, 0.8, geometry.max_error_m as f32, 0.0),
        tree: UVec4::new(geometry.tree.values.len() as u32, 0, 0, 0),
    };
    let page = storage("lod page", &uniform_bytes(&page), wgpu::BufferUsages::UNIFORM);
    let tree = storage("lod tree", bytemuck::cast_slice(&geometry.tree.values), wgpu::BufferUsages::STORAGE);
    let order = storage(
        "lod order",
        bytemuck::cast_slice(&geometry.hierarchy.refinement.order.values),
        wgpu::BufferUsages::STORAGE,
    );
    let positions = storage("lod positions", bytemuck::cast_slice(&geometry.positions.values), wgpu::BufferUsages::STORAGE);
    let head = storage("lod head", &[0; 64], rw);
    let table = storage("lod table", &[0; 4096], rw);
    let dispatch = storage("lod dispatch", &[0; 16], rw | wgpu::BufferUsages::INDIRECT);
    let states = storage("lod states", bytemuck::cast_slice(&before.states), rw);
    let pipeline = |entry| {
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: None,
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        })
    };
    let names = ["measure_nodes", "choose", "emit", "accept"];
    let entries = names.map(pipeline);
    let all: [&[(u32, &wgpu::Buffer)]; 3] = [
        &[(0, &view), (1, &bins)],
        &[(0, &page), (1, &tree), (2, &order), (3, &positions)],
        &[(0, &head), (1, &table), (2, &dispatch), (3, &states)],
    ];
    // Auto layouts hold only the bindings an entry point uses.
    let used = |entry: &str| -> [&[u32]; 3] {
        match entry {
            "measure_nodes" => [&[0, 1], &[0, 1], &[3]],
            "choose" => [&[0, 1], &[], &[]],
            "emit" => [&[0, 1], &[0, 1], &[0, 1, 2, 3]],
            _ => [&[], &[0, 2, 3], &[0, 1]],
        }
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        for (pipeline, entry) in entries.iter().zip(names) {
            pass.set_pipeline(pipeline);
            for (group, bindings) in used(entry).into_iter().enumerate().filter(|(_, b)| !b.is_empty()) {
                let entries: Vec<_> = all[group]
                    .iter()
                    .filter(|(binding, _)| bindings.contains(binding))
                    .map(|(binding, buffer)| wgpu::BindGroupEntry { binding: *binding, resource: buffer.as_entire_binding() })
                    .collect();
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &pipeline.get_bind_group_layout(group as u32),
                    entries: &entries,
                });
                pass.set_bind_group(group as u32, &bind, &[]);
            }
            match entry {
                "measure_nodes" => pass.dispatch_workgroups((geometry.tree.values.len() as u32).div_ceil(256), 1, 1),
                "accept" => pass.dispatch_workgroups_indirect(&dispatch, 0),
                _ => pass.dispatch_workgroups(1, 1, 1),
            }
        }
    }
    let read = |buffer: &wgpu::Buffer| {
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: buffer.size(),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        (buffer.size(), staging)
    };
    let (head_size, head_read) = read(&head);
    let (table_size, table_read) = read(&table);
    let (bins_size, bins_read) = read(&bins);
    let (states_size, states_read) = read(&states);
    encoder.copy_buffer_to_buffer(&head, 0, &head_read, 0, head_size);
    encoder.copy_buffer_to_buffer(&table, 0, &table_read, 0, table_size);
    encoder.copy_buffer_to_buffer(&bins, 0, &bins_read, 0, bins_size);
    encoder.copy_buffer_to_buffer(&states, 0, &states_read, 0, states_size);
    queue.submit([encoder.finish()]);
    for staging in [&head_read, &table_read, &bins_read, &states_read] {
        staging.slice(..).map_async(wgpu::MapMode::Read, |result| result.unwrap());
    }
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let words = |staging: &wgpu::Buffer| -> Vec<u32> {
        bytemuck::cast_slice(&staging.slice(..).get_mapped_range().expect("mapped GPU buffer range")).to_vec()
    };
    let head = words(&head_read);
    let table: Vec<[u32; 2]> = words(&table_read)
        .chunks_exact(2)
        .take(geometry.tree.values.len())
        .map(|pair| [pair[0], pair[1]])
        .collect();
    let selected = head[1];
    let order = &geometry.hierarchy.refinement.order.values;
    let ordinals = (0..selected)
        .map(|instance| {
            let node = table.partition_point(|entry| entry[0] <= instance) - 1;
            order[(table[node][1] + instance - table[node][0]) as usize]
        })
        .collect();
    let bins = words(&bins_read);
    let points = bins[..161]
        .iter()
        .scan(0i32, |total, diff| {
            *total += *diff as i32;
            Some(*total)
        })
        .collect();
    Run {
        selected,
        accepted: head[7],
        table,
        ordinals,
        chosen: head[8],
        points,
        next: Frame { states: words(&states_read), chosen: bins[161] },
    }
}

fn surface(budget: &CpuBudget, side: usize) -> std::sync::Arc<Geometry> {
    let points: Vec<_> = (0..side * side)
        .map(|i| [(i % side) as f64 * 0.05, (i / side) as f64 * 0.05, ((i % 13) as f64 * 0.01).sin()])
        .collect();
    let (schema, page) = fixture(&points, vec![]);
    Geometry::prepare(budget, &schema, stored(budget, &schema, &page, [0.0; 3])).unwrap()
}

/// Looking straight down on the page's centre from `height` metres.
fn camera(height: f32) -> Mat4 {
    let centre = Vec3::new(5.0, 5.0, 0.0);
    Mat4::perspective_rh(1.0, 800.0 / 600.0, 0.1, 1000.0)
        * Mat4::look_at_rh(centre + Vec3::new(0.0, 0.0, height), centre, Vec3::Y)
}

/// Each leaf is drawn by at most one node on its path to the root, and when the whole page
/// is in view by exactly one: no overlap, and no hole.
fn assert_cut(geometry: &Geometry, run: &Run, whole: bool) {
    let tree = &geometry.tree.values;
    let counts: Vec<u32> = (0..tree.len())
        .map(|node| {
            let next = run.table.get(node + 1).map_or(run.selected, |entry| entry[0]);
            next - run.table[node][0]
        })
        .collect();
    for (index, node) in tree.iter().enumerate().filter(|(_, node)| node.leaf == 1) {
        let mut drawn = 0;
        let mut at = index as u32;
        loop {
            drawn += u32::from(counts[at as usize] > 0);
            if tree[at as usize].parent == ROOT {
                break;
            }
            at = tree[at as usize].parent;
        }
        assert!(drawn <= 1, "leaf {index} drawn by {drawn} nodes");
        assert!(!whole || drawn == 1 || node.count == 0, "leaf {index} left as a hole");
    }
    let unique: BTreeSet<_> = run.ordinals.iter().collect();
    assert_eq!(unique.len(), run.ordinals.len(), "a point is listed twice");
    assert!(run.ordinals.iter().all(|ordinal| (*ordinal as usize) < geometry.len()));
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-gpu"]
fn gpu_lod_lists_a_hole_free_cut_within_the_point_budget() {
    let budget = CpuBudget::new(64 << 20);
    let geometry = surface(&budget, 200);
    let full = run(&geometry, camera(12.0), 1 << 20, &first());
    assert_cut(&geometry, &full, true);
    assert_eq!(full.selected as usize, geometry.len(), "a whole close view within budget draws every point");
    assert_eq!(full.accepted, full.selected);
    for (height, points) in [(12.0, 4000), (40.0, 600), (300.0, 50)] {
        let limited = run(&geometry, camera(height), points, &first());
        assert_cut(&geometry, &limited, true);
        assert!(limited.selected > 0 && limited.selected <= points, "{height} m: {} of {points}", limited.selected);
    }
    let partial = run(&geometry, camera(3.0), 4000, &first());
    assert_cut(&geometry, &partial, false);
    assert!(partial.selected > 0 && partial.selected <= 4000);
    let far = run(&geometry, camera(300.0), 1 << 20, &first());
    assert!(far.selected < full.selected, "distant views select fewer points");
    let behind = run(&geometry, camera(12.0) * Mat4::from_translation(Vec3::new(0.0, 0.0, 50.0)), 1 << 20, &first());
    assert_eq!(behind.selected, 0, "a page outside the view lists nothing");
}

fn ordinals(run: &Run) -> BTreeSet<u32> {
    run.ordinals.iter().copied().collect()
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-gpu"]
fn gpu_lod_keeps_a_refined_cut_until_its_error_leaves_the_dead_band() {
    let budget = CpuBudget::new(64 << 20);
    let geometry = surface(&budget, 200);
    let near = run(&geometry, camera(100.0), 1 << 20, &first());
    assert!(near.selected < geometry.len() as u32, "the view must show a partial cut");
    let fresh = run(&geometry, camera(110.0), 1 << 20, &first());
    assert!(fresh.selected < near.selected, "without memory 10% further coarsens the cut");
    let kept = run(&geometry, camera(110.0), 1 << 20, &near.next);
    assert_cut(&geometry, &kept, true);
    assert_eq!(ordinals(&kept), ordinals(&near), "10% further keeps last frame's cut");
    let back = run(&geometry, camera(100.0), 1 << 20, &kept.next);
    assert_eq!(ordinals(&back), ordinals(&near), "returning keeps the same cut");
    let far = run(&geometry, camera(150.0), 1 << 20, &near.next);
    assert_cut(&geometry, &far, true);
    assert!(far.selected < near.selected, "50% further leaves the dead band and coarsens");
    let close = run(&geometry, camera(90.0), 1 << 20, &near.next);
    assert_cut(&geometry, &close, true);
    assert!(close.selected > near.selected, "coming closer refines at once");
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-gpu"]
fn gpu_lod_keeps_last_frames_threshold_until_two_bins_finer_fit() {
    let budget = CpuBudget::new(64 << 20);
    let geometry = surface(&budget, 200);
    let view = camera(12.0);
    let points = run(&geometry, view, 1 << 20, &first()).points;
    // A threshold bin with three finer bins that each draw more points.
    let bin = (FINEST as usize + 3..points.len())
        .find(|&bin| points[bin - 3..=bin].windows(2).all(|pair| pair[0] > pair[1]) && points[bin] > 0)
        .expect("a budget-limited bin") as u32;
    let fits = |bin: u32| points[bin as usize] as u32;
    let held = |chosen| Frame { chosen, ..first() };
    assert_eq!(run(&geometry, view, fits(bin), &first()).chosen, bin);
    assert_eq!(run(&geometry, view, fits(bin - 1), &first()).chosen, bin - 1);
    let kept = run(&geometry, view, fits(bin - 1), &held(bin));
    assert_eq!((kept.chosen, kept.selected), (bin, fits(bin)), "one bin finer keeps last frame's");
    assert_eq!(run(&geometry, view, fits(bin - 2), &held(bin)).chosen, bin - 2, "two bins finer refines");
    assert_eq!(run(&geometry, view, fits(bin), &held(bin - 1)).chosen, bin, "the budget coarsens at once");
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-gpu"]
fn gpu_lod_cut_stays_hole_free_from_any_node_states() {
    let budget = CpuBudget::new(64 << 20);
    let geometry = surface(&budget, 200);
    let mut seed = 0x9e37_79b9u32;
    let states = (0..64)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (0..8).fold(0, |word, nibble| word | ((seed >> (nibble * 3)) % 9) << (nibble * 4))
        })
        .collect();
    let scrambled = Frame { states, chosen: 0 };
    for height in [12.0, 40.0, 100.0, 150.0, 300.0] {
        let cut = run(&geometry, camera(height), 1 << 20, &scrambled);
        assert_cut(&geometry, &cut, true);
    }
}
