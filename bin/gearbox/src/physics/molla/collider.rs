use super::*;

pub(super) struct ColliderAccess {
    pub shared: Shared,
    pub handle: rt::ColliderHandle,
    pub entity: Option<Entity>,
}

impl ColliderAccess {
    fn edit(
        &mut self,
        change: impl FnOnce(&mut rt::ColliderDesc) -> molla_core::Result<()>,
        recompute: bool,
    ) {
        let mut world = self.shared.world();
        let mut desc = world
            .scene
            .collider(self.handle)
            .expect("collider handle")
            .clone();
        let parent = desc.parent;
        let result =
            change(&mut desc).and_then(|()| world.scene.replace_collider(self.handle, desc));
        if result.is_ok()
            && recompute
            && let Some(parent) = parent
        {
            apply(world.recompute_body_mass(parent));
        }
        apply(result);
    }
}

impl Collider for ColliderAccess {
    fn position(&self) -> Pose {
        convert::pose(
            self.shared
                .world()
                .scene
                .collider_world_pose(self.handle)
                .expect("collider pose"),
        )
    }
    fn position_wrt_parent(&self) -> Option<Pose> {
        let world = self.shared.world();
        let desc = world.scene.collider(self.handle)?;
        desc.parent.map(|_| convert::pose(desc.pose))
    }
    fn parent(&self) -> Option<BodyId> {
        self.shared
            .world()
            .scene
            .collider(self.handle)?
            .parent
            .map(|b| BodyId(b.to_bits()))
    }
    fn shape(&self) -> ShapeView {
        convert::shape_view(
            self.shared
                .world()
                .collider_shape_view(self.handle)
                .expect("collider geometry"),
        )
    }
    fn local_aabb(&self) -> Aabb {
        convert::bounds(
            self.shared
                .world()
                .collider_local_aabb(self.handle)
                .expect("collider bounds"),
        )
    }
    fn aabb(&self) -> Aabb {
        convert::bounds(
            self.shared
                .world()
                .collider_world_aabb(self.handle)
                .expect("collider bounds"),
        )
    }
    fn aabb_at(&self, pose: Pose) -> Aabb {
        convert::bounds(
            self.shared
                .world()
                .collider_aabb_at_pose(self.handle, convert::transform(pose))
                .expect("collider bounds"),
        )
    }
    fn is_sensor(&self) -> bool {
        self.shared
            .world()
            .scene
            .collider(self.handle)
            .expect("collider handle")
            .sensor
    }
    fn groups(&self) -> CollisionGroups {
        let world = self.shared.world();
        let desc = world.scene.collider(self.handle).expect("collider handle");
        CollisionGroups {
            memberships: desc.memberships,
            filter: desc.filter,
        }
    }
    fn friction(&self) -> f64 {
        self.shared
            .world()
            .scene
            .collider(self.handle)
            .expect("collider handle")
            .friction
    }
    fn restitution(&self) -> f64 {
        self.shared
            .world()
            .scene
            .collider(self.handle)
            .expect("collider handle")
            .restitution
    }
    fn mass(&self) -> f64 {
        self.shared
            .world()
            .collider_mass_properties(self.handle)
            .expect("collider mass")
            .mass
    }
    fn is_enabled(&self) -> bool {
        self.shared
            .world()
            .scene
            .collider_is_enabled(self.handle)
            .expect("collider handle")
    }
    fn entity(&self) -> Option<Entity> {
        self.entity
    }
}

impl ColliderMut for ColliderAccess {
    fn set_shape(&mut self, shape: Shape) {
        self.edit(
            |c| {
                c.geometry = convert::geometry(shape)?;
                Ok(())
            },
            true,
        );
    }
    fn set_position_wrt_parent(&mut self, pose: Pose) {
        self.edit(
            |c| {
                c.pose = convert::transform(pose);
                Ok(())
            },
            true,
        );
    }
    fn set_position(&mut self, pose: Pose) {
        let mut world = self.shared.world();
        apply(
            world
                .scene
                .set_collider_world_pose(self.handle, convert::transform(pose)),
        );
        if let Some(parent) = world.scene.collider(self.handle).and_then(|c| c.parent) {
            apply(world.recompute_body_mass(parent));
        }
    }
    fn set_friction(&mut self, friction: f64) {
        if self.friction() == friction {
            return;
        }
        self.edit(
            |c| {
                c.friction = friction;
                Ok(())
            },
            false,
        );
    }
    fn set_restitution(&mut self, restitution: f64) {
        if self.restitution() == restitution {
            return;
        }
        self.edit(
            |c| {
                c.restitution = restitution;
                Ok(())
            },
            false,
        );
    }
    fn set_friction_combine_rule(&mut self, rule: CombineRule) {
        if self
            .shared
            .world()
            .scene
            .collider(self.handle)
            .expect("collider handle")
            .contact_material
            .friction_combine
            == convert::combine(rule)
        {
            return;
        }
        self.edit(
            |c| {
                c.contact_material.friction_combine = convert::combine(rule);
                Ok(())
            },
            false,
        );
    }
    fn set_density(&mut self, density: f64) {
        self.edit(
            |c| {
                c.mass = sim::ColliderMass::Density(density);
                Ok(())
            },
            true,
        );
    }
    fn set_mass(&mut self, mass: f64) {
        self.edit(
            |c| {
                c.mass = sim::ColliderMass::Mass(mass);
                Ok(())
            },
            true,
        );
    }
    fn set_groups(&mut self, groups: CollisionGroups) {
        self.edit(
            |c| {
                c.memberships = groups.memberships;
                c.filter = groups.filter;
                Ok(())
            },
            false,
        );
    }
    fn set_enabled(&mut self, enabled: bool) {
        self.edit(
            |c| {
                c.enabled = enabled;
                Ok(())
            },
            true,
        );
    }
}
