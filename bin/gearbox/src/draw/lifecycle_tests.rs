use super::*;
use gearbox_api::GridCells;

#[path = "legacy_admission_tests.rs"]
mod compact_admission;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
        .init_asset::<Mesh>()
        .init_asset::<Image>()
        .init_asset::<StrokeMaterial>()
        .init_asset::<SurfaceMaterial>()
        .init_asset::<heat::HeatMaterial>()
        .init_resource::<DrawStoreRes>()
        .init_resource::<DrawVisuals>()
        .add_systems(Update, sync_draws);
    let home = app.world_mut().spawn_empty().id();
    let frame = gearbox_globe::Datum::at(52.370216, 4.895168);
    app.insert_resource(crate::globe::Sites {
        root: home,
        list: vec![crate::globe::SiteEntry {
            entity: home,
            name: "home".into(),
            frame,
        }],
        current: 0,
        land: gearbox_globe::Terrain::new(&frame),
        spare: Vec::new(),
    });
    let lut = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::default());
    app.insert_resource(TonemappingLuts {
        agx: lut.clone(),
        blender_filmic: lut.clone(),
        tony_mc_mapface: lut,
    });
    app
}

fn settle(app: &mut App) {
    for _ in 0..5 {
        app.update();
    }
}

fn apply(app: &App, entity: DrawEntity) {
    app.world()
        .resource::<DrawStoreRes>()
        .lock()
        .apply(entity, 0.0)
        .unwrap();
}

fn counts(app: &mut App) -> [usize; 6] {
    let world = app.world_mut();
    [
        world.query::<&DrawRoot>().iter(world).count(),
        world.resource::<Assets<Mesh>>().len(),
        world.resource::<Assets<Image>>().len(),
        world.resource::<Assets<heat::HeatMaterial>>().len(),
        world.resource::<Assets<StrokeMaterial>>().len(),
        world.resource::<Assets<SurfaceMaterial>>().len(),
    ]
}

fn populate(app: &App, step: u8) {
    apply(
        app,
        DrawEntity::lines(
            "lifetime",
            "line",
            &[[0.0; 3], [1.0 + f32::from(step), 0.0, 0.0]],
        ),
    );
    apply(
        app,
        DrawEntity::triangles(
            "lifetime",
            "surface",
            &[[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        )
        .with_color(gearbox_api::wire::draw::rgba(step, 210, 80, 120)),
    );
    apply(
        app,
        DrawEntity::grid("lifetime", "grid", 2, 2, 0.5, GridCells::U8(vec![step; 4])),
    );
}

#[test]
fn repeated_updates_reuse_meshes_and_release_replaced_grid_images() {
    let mut app = app();
    populate(&app, 1);
    settle(&mut app);
    let baseline = counts(&mut app);
    assert_eq!(&baseline[..4], &[3, 3, 3, 1]);
    let original: Vec<_> = app
        .world()
        .resource::<DrawVisuals>()
        .map
        .values()
        .flat_map(|v| v.parts.iter().flatten().chain(v.heat.iter()))
        .map(|part| part.mesh.id())
        .collect();
    for step in 2..=64 {
        populate(&app, step);
        settle(&mut app);
        assert_eq!(counts(&mut app), baseline, "asset growth at update {step}");
        let meshes = app.world().resource::<Assets<Mesh>>();
        assert!(original.iter().all(|id| meshes.contains(*id)));
    }
    apply(&app, DrawEntity::clear("lifetime"));
    settle(&mut app);
    assert_eq!(counts(&mut app), [0, 0, 1, 0, baseline[4], baseline[5]]);
    assert!(app.world().resource::<DrawVisuals>().map.is_empty());
    app.world_mut().resource_mut::<DrawVisuals>().materials = MaterialCache::default();
    settle(&mut app);
    assert_eq!(counts(&mut app), [0, 0, 1, 0, 0, 0]);
}

#[test]
fn view_only_draws_move_to_the_viewer_overlay_and_back() {
    let mut app = app();
    let line = || DrawEntity::lines("view", "line", &[[0.0; 3], [1.0, 0.0, 0.0]]).with_width(0.1);
    let layers = |app: &mut App| {
        let world = app.world_mut();
        let parts: Vec<Entity> = world.resource::<DrawVisuals>().map.values()
            .flat_map(|v| v.parts.iter().flatten().map(|part| part.entity)).collect();
        parts.into_iter().map(|e| world.get::<RenderLayers>(e).cloned().unwrap()).collect::<Vec<_>>()
    };
    let overlay = RenderLayers::layer(crate::viewer::recorder::OVERLAY_LAYER);
    apply(&app, line().with_flags(draw_flag::VIEW_ONLY));
    settle(&mut app);
    assert_eq!(layers(&mut app), [overlay.clone()], "a view-only line stays out of captures");
    apply(&app, line().with_color(gearbox_api::wire::draw::rgba(9, 9, 9, 255)));
    settle(&mut app);
    assert_eq!(layers(&mut app), [RenderLayers::layer(DRAW_LAYER)], "clearing the flag puts it back");
    apply(&app, line().with_flags(draw_flag::VIEW_ONLY));
    settle(&mut app);
    assert_eq!(layers(&mut app), [overlay], "setting it again moves the kept part");
}

#[test]
fn drawings_show_at_half_opacity_unless_they_ask_otherwise() {
    let mut app = app();
    let send = |app: &App, opacity: f32| {
        let corners = [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let solid = gearbox_api::wire::draw::rgba(200, 80, 40, 255);
        apply(app, DrawEntity::triangles("see", "solid", &corners).with_style(gearbox_api::draw_style::PLAIN)
            .with_color(solid).with_opacity(opacity));
        apply(app, DrawEntity::lines("see", "line", &corners).with_width(0.1).with_opacity(opacity));
        apply(app, DrawEntity::grid("see", "grid", 2, 2, 0.5, GridCells::U8(vec![9; 4])).with_opacity(opacity));
    };
    let shown = |app: &mut App| {
        let world = app.world();
        let visuals = &world.resource::<DrawVisuals>().map;
        let visual = |id: &str| &visuals[&("see".to_string(), id.to_string())];
        let part = |id: &str, slot: usize| visual(id).parts[slot].as_ref().unwrap().entity;
        let fill = &world.get::<MeshMaterial3d<SurfaceMaterial>>(part("solid", 0)).unwrap().0;
        let fill = world.resource::<Assets<SurfaceMaterial>>().get(fill).unwrap();
        let stroke = &world.get::<MeshMaterial3d<StrokeMaterial>>(part("line", 1)).unwrap().0;
        let stroke = world.resource::<Assets<StrokeMaterial>>().get(stroke).unwrap();
        let heat = visual("grid").heat_material.as_ref().unwrap();
        let heat = world.resource::<Assets<heat::HeatMaterial>>().get(heat).unwrap();
        (fill.flags.z, fill.translucent, stroke.finish.y, heat.params.frost.y, visual("line").opacity)
    };
    let near = |o: f32, want: f32| (o - want).abs() <= 1.0 / 255.0;
    for (given, asked) in [(0.0, 0.5), (0.2, 0.2)] {
        send(&app, given);
        settle(&mut app);
        let (fill, translucent, stroke, heat, labels) = shown(&mut app);
        let scene = [fill, stroke, heat];
        let want = visual::scene_opacity(asked);
        assert!(scene.iter().all(|o| near(*o, want)), "scene coverage for {asked}: {scene:?}");
        assert!(near(labels, asked), "labels are painted at {asked} itself, not {labels}");
        assert!(translucent, "a solid fill blends once it is see-through");
    }
    assert!(near(visual::scene_opacity(0.5), 0.2176), "half shown is about a fifth in the scene");
    send(&app, 1.0);
    settle(&mut app);
    assert_eq!(shown(&mut app), (1.0, false, 1.0, 1.0, 1.0), "asked for fully opaque");
}

#[test]
fn removed_roots_recover_and_kind_replacement_releases_old_parts() {
    let mut app = app();
    populate(&app, 1);
    settle(&mut app);
    let key = ("lifetime".into(), "grid".into());
    let root = app.world().resource::<DrawVisuals>().map[&key].root;
    app.world_mut().entity_mut(root).despawn();
    settle(&mut app);
    assert_ne!(app.world().resource::<DrawVisuals>().map[&key].root, root);
    assert_eq!(&counts(&mut app)[..4], &[3, 3, 3, 1]);
    apply(
        &app,
        DrawEntity::text("lifetime", "grid", &[[0.0; 3]], "label").with_lifetime(1.0),
    );
    settle(&mut app);
    assert_eq!(&counts(&mut app)[..4], &[3, 2, 1, 0]);
    assert_eq!(app.world().resource::<DrawStoreRes>().lock().expire(2.0), 1);
    settle(&mut app);
    assert_eq!(&counts(&mut app)[..4], &[2, 2, 1, 0]);
}

fn ghost_app() -> (App, Entity, Entity) {
    let mut app = app();
    app.init_asset::<ghost::GhostMaterial>()
        .add_systems(Update, ghost::build_ghosts.after(sync_draws));
    let (stage, bodies) = crate::controller::spawn_fleet(app.world_mut(), &["Probe"]);
    let body = bodies[0];
    let prim = app.world().get::<ChildOf>(body).unwrap().parent();
    for entity in [stage, prim, body] {
        app.world_mut()
            .entity_mut(entity)
            .insert(GlobalTransform::IDENTITY);
    }
    apply(
        &app,
        DrawEntity::ghost(
            "lifetime",
            "robot",
            "probe",
            &[gearbox_api::Instance::default(); 2],
        ),
    );
    (app, stage, body)
}

fn ghost_part(app: &mut App, body: Entity, name: &str) -> Entity {
    let mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Mesh::from(Cuboid::default()));
    app.world_mut()
        .spawn((
            usd_bevy::UsdPrimRef::new(format!("/Fleet/Probe/body/{name}")),
            Mesh3d(mesh),
            GlobalTransform::IDENTITY,
            Visibility::Inherited,
            ChildOf(body),
        ))
        .id()
}

fn copies(app: &App) -> Vec<Entity> {
    app.world().resource::<DrawVisuals>().map[&("lifetime".into(), "robot".into())]
        .ghost_copies
        .clone()
}

#[test]
fn ghosts_refresh_late_parts_without_following_ordinary_robot_motion() {
    let (mut app, stage, body) = ghost_app();
    let first = ghost_part(&mut app, body, "First");
    settle(&mut app);
    assert_eq!(copies(&app).len(), 2);
    let original = copies(&app);
    app.world_mut()
        .entity_mut(first)
        .insert(GlobalTransform::from_translation(Vec3::X * 3.0));
    settle(&mut app);
    assert_eq!(
        copies(&app),
        original,
        "ordinary articulation must not rebuild frozen poses"
    );
    let second = ghost_part(&mut app, body, "Second");
    settle(&mut app);
    assert_eq!(
        copies(&app).len(),
        4,
        "late model parts must appear in every pose"
    );
    app.world_mut()
        .entity_mut(second)
        .insert(Visibility::Hidden);
    settle(&mut app);
    assert_eq!(copies(&app).len(), 2);
    app.world_mut()
        .entity_mut(second)
        .insert(Visibility::Inherited);
    settle(&mut app);
    assert_eq!(copies(&app).len(), 4);
    app.world_mut().entity_mut(second).despawn();
    settle(&mut app);
    assert_eq!(copies(&app).len(), 2);
    assert_eq!(
        app.world().resource::<Assets<ghost::GhostMaterial>>().len(),
        2
    );
    app.world_mut().entity_mut(stage).despawn();
    settle(&mut app);
    assert!(copies(&app).is_empty());
    settle(&mut app);
    assert_eq!(
        app.world().resource::<Assets<ghost::GhostMaterial>>().len(),
        0
    );
    assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 0);
}

#[test]
fn ghosts_wait_for_mesh_data_and_recover_removed_copies() {
    let (mut app, _, body) = ghost_app();
    let part = ghost_part(&mut app, body, "Delayed");
    let handle = app.world().get::<Mesh3d>(part).unwrap().0.clone();
    let mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .remove(handle.id())
        .unwrap();
    settle(&mut app);
    assert!(copies(&app).is_empty());
    app.world_mut()
        .resource_mut::<Assets<Mesh>>()
        .insert(&handle, mesh)
        .unwrap();
    settle(&mut app);
    assert_eq!(copies(&app).len(), 2);
    let removed = copies(&app)[0];
    app.world_mut().entity_mut(removed).despawn();
    settle(&mut app);
    assert_eq!(copies(&app).len(), 2);
    assert!(
        copies(&app)
            .iter()
            .all(|copy| app.world().get_entity(*copy).is_ok())
    );
    apply(&app, DrawEntity::delete("lifetime", "robot"));
    settle(&mut app);
    assert_eq!(
        app.world().resource::<Assets<ghost::GhostMaterial>>().len(),
        0
    );
}
