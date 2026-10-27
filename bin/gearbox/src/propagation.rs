//! Low-precision transform propagation under big_space grids that walks only
//! the subtrees Bevy's `mark_dirty_trees` flagged, in place of big_space's
//! pass over every descendant of every low-precision root.

use bevy::prelude::*;
use bevy::transform::TransformSystems;
use bevy::transform::components::TransformTreeChanged;
use bevy::transform::systems::{StaticTransformOptimizations, mark_dirty_trees};
use big_space::grid::propagation::LowPrecisionRoot;
use big_space::prelude::*;

pub struct DirtyPropagationPlugin;

impl Plugin for DirtyPropagationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StaticTransformOptimizations>()
            .configure_sets(PostUpdate, BigSpaceSystems::PropagateLowPrecision.run_if(|| false))
            .add_systems(
                PostUpdate,
                (mark_dirty_trees, propagate_low_precision)
                    .chain()
                    .in_set(TransformSystems::Propagate)
                    .after(BigSpaceSystems::PropagateHighPrecision)
                    .after(Grid::tag_low_precision_roots)
                    .before(BigSpaceSystems::PropagateLowPrecision),
            );
    }
}

type Tree<'w, 's> = Query<
    'w,
    's,
    (
        Ref<'static, Transform>,
        &'static mut GlobalTransform,
        Ref<'static, TransformTreeChanged>,
        Option<&'static Children>,
    ),
    (With<ChildOf>, Without<CellCoord>, Without<Grid>),
>;

type Links<'w, 's> = Query<
    'w,
    's,
    (Entity, Ref<'static, ChildOf>),
    (With<Transform>, With<GlobalTransform>, Without<CellCoord>, Without<Grid>),
>;

fn propagate_low_precision(
    root_parents: Query<Ref<GlobalTransform>, Or<(With<Grid>, With<CellCoord>)>>,
    roots: Query<(Entity, &ChildOf), With<LowPrecisionRoot>>,
    tree: Tree,
    links: Links,
) {
    roots.par_iter().for_each(|(root, parent)| {
        if let Ok(parent_transform) = root_parents.get(parent.parent()) {
            // SAFETY: each root is unique and the hierarchy is a forest, so no
            // two walks reach the same entity.
            unsafe { walk(&parent_transform, &tree, &links, root, parent_transform.is_changed()) };
        }
    });
}

/// Recomputes `entity` and its descendants below a changed parent, and descends
/// into a subtree only when its parent changed or it carries a dirty mark.
///
/// # Safety
///
/// No other fetch from `tree` may reach `entity` or its descendants while this
/// runs, and the hierarchy below `entity` must be a tree.
unsafe fn walk(parent: &GlobalTransform, tree: &Tree, links: &Links, entity: Entity, mut changed: bool) {
    // SAFETY: the caller rules out every other fetch of this entity.
    let Ok((transform, mut global, dirty, children)) = (unsafe { tree.get_unchecked(entity) }) else {
        return;
    };
    if !changed && !dirty.is_changed() {
        return;
    }
    changed |= transform.is_changed() || global.is_added();
    if changed {
        *global = parent.mul_transform(*transform);
    }
    let Some(children) = children else { return };
    for (child, child_of) in links.iter_many(children) {
        assert_eq!(child_of.parent(), entity, "malformed hierarchy below {entity}");
        // SAFETY: each child has this entity as its only parent.
        unsafe { walk(global.as_ref(), tree, links, child, changed || child_of.is_changed()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use big_space::plugin::BigSpaceMinimalPlugins;

    #[derive(Component)]
    struct Mark(&'static str);

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((BigSpaceMinimalPlugins, DirtyPropagationPlugin)).add_systems(Startup, |mut commands: Commands| {
            commands.spawn_big_space_default(|root| {
                root.spawn_spatial(FloatingOrigin);
                root.spawn_spatial((Transform::from_xyz(100.0, 0.0, 0.0), Mark("cell"))).with_children(|cell| {
                    cell.spawn((Transform::from_xyz(10.0, 0.0, 0.0), Mark("arm"))).with_children(|arm| {
                        arm.spawn((Transform::from_xyz(1.0, 0.0, 0.0), Mark("hand")));
                    });
                    cell.spawn((Transform::from_xyz(0.0, 5.0, 0.0), Mark("still")));
                });
            });
        });
        app.update();
        app.update();
        app
    }

    fn entity(app: &mut App, name: &str) -> Entity {
        let mut marks = app.world_mut().query::<(Entity, &Mark)>();
        marks.iter(app.world()).find(|(_, m)| m.0 == name).unwrap().0
    }

    fn at(app: &mut App, name: &str) -> Vec3 {
        let e = entity(app, name);
        app.world().get::<GlobalTransform>(e).unwrap().translation()
    }

    #[test]
    fn a_moved_leaf_parent_or_cell_reaches_every_global_transform_below_it() {
        let mut app = app();
        assert_eq!(at(&mut app, "hand"), Vec3::new(111.0, 0.0, 0.0));
        assert_eq!(at(&mut app, "still"), Vec3::new(100.0, 5.0, 0.0));
        let hand = entity(&mut app, "hand");
        app.world_mut().get_mut::<Transform>(hand).unwrap().translation.x = 2.0;
        app.update();
        assert_eq!(at(&mut app, "hand"), Vec3::new(112.0, 0.0, 0.0));
        let arm = entity(&mut app, "arm");
        app.world_mut().get_mut::<Transform>(arm).unwrap().translation.y = 3.0;
        app.update();
        assert_eq!(at(&mut app, "hand"), Vec3::new(112.0, 3.0, 0.0));
        let cell = entity(&mut app, "cell");
        app.world_mut().get_mut::<Transform>(cell).unwrap().translation.z = 7.0;
        app.update();
        assert_eq!(at(&mut app, "hand"), Vec3::new(112.0, 3.0, 7.0));
        assert_eq!(at(&mut app, "still"), Vec3::new(100.0, 5.0, 7.0));
    }

    #[test]
    fn a_reparented_entity_takes_its_new_parent_s_place() {
        let mut app = app();
        let (hand, still) = (entity(&mut app, "hand"), entity(&mut app, "still"));
        app.world_mut().entity_mut(hand).insert(ChildOf(still));
        app.update();
        assert_eq!(at(&mut app, "hand"), Vec3::new(101.0, 5.0, 0.0));
    }
}
