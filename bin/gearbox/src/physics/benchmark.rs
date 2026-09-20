use super::*;

pub(crate) fn project(app: &mut App, stage: &openusd::usd::Stage) -> Entity {
    let root = app.world_mut().spawn(Transform::IDENTITY).id();
    let prims = usd_bevy::live::project_stage_under(app.world_mut(), stage, root);
    let meta = attach::read_stage_meta(stage);
    let mut pending = attach::PendingPhysics::default();
    let mut ordered: Vec<_> = prims.iter().collect();
    ordered.sort_by_key(|(path, _)| *path);
    for (path, entity) in ordered {
        if path != "/" {
            app.world_mut().entity_mut(entity).insert(Name::new(path.to_string()));
            attach::attach_physics_to_entity(app.world_mut(), entity, &mut pending, &meta);
        }
    }
    let paths = prims.iter().map(|(path, entity)| (path.to_string(), entity)).collect();
    attach::resolve_pending_physics(app.world_mut(), &pending, &paths);
    attach::populate_articulation_joints(app.world_mut(), &pending.articulation_roots);
    app.update();
    let mut convert = bevy::ecs::schedule::Schedule::default();
    convert.add_systems((
        bodies::convert_rigid_bodies,
        colliders::convert_colliders,
        colliders::apply_physics_materials,
        colliders::apply_collision_filters,
        joints::convert_joints,
    ).chain());
    convert.run(app.world_mut());
    root
}
