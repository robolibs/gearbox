use super::*;

pub(super) fn transform(pose: Pose) -> Transform {
    Transform::new(pose.translation, pose.rotation)
}
pub(super) fn pose(value: Transform) -> Pose {
    Pose::new(value.position, value.rotation)
}
pub(super) fn bounds(value: molla_geometry::Aabb) -> Aabb {
    Aabb {
        mins: value.min,
        maxs: value.max,
    }
}

pub(super) fn mass(props: MassProps) -> sim::MassProperties {
    sim::MassProperties {
        mass: props.mass,
        local_com: props.local_com,
        inertia: match props.inertia {
            Inertia::Principal(moments) => DMat3::from_diagonal(moments),
            Inertia::Tensor(tensor) => tensor,
        },
    }
}

pub(super) fn body_kind(kind: BodyKind) -> rt::BodyKind {
    match kind {
        BodyKind::Dynamic => rt::BodyKind::Dynamic,
        BodyKind::Fixed => rt::BodyKind::Fixed,
        BodyKind::Kinematic => rt::BodyKind::Kinematic,
    }
}

pub(super) fn combine(rule: CombineRule) -> sim::CoefficientCombineRule {
    match rule {
        CombineRule::Average => sim::CoefficientCombineRule::Average,
        CombineRule::Min => sim::CoefficientCombineRule::Min,
        CombineRule::Multiply => sim::CoefficientCombineRule::Multiply,
        CombineRule::Max => sim::CoefficientCombineRule::Max,
    }
}

pub(super) fn axis(axis: JointAxis) -> jc::JointAxis {
    match axis {
        JointAxis::LinX => jc::JointAxis::LinX,
        JointAxis::LinY => jc::JointAxis::LinY,
        JointAxis::LinZ => jc::JointAxis::LinZ,
        JointAxis::AngX => jc::JointAxis::AngX,
        JointAxis::AngY => jc::JointAxis::AngY,
        JointAxis::AngZ => jc::JointAxis::AngZ,
    }
}

pub(super) fn geometry(shape: Shape) -> molla_core::Result<rt::ColliderGeometry> {
    Ok(match shape {
        Shape::Cuboid { half_extents } => rt::ColliderGeometry::Cuboid { half_extents },
        Shape::Ball { radius } => rt::ColliderGeometry::Ball { radius },
        Shape::Capsule { a, b, radius } => rt::ColliderGeometry::CapsuleEndpoints { a, b, radius },
        Shape::Cylinder {
            half_height,
            radius,
        } => rt::ColliderGeometry::Cylinder {
            half_height,
            radius,
        },
        Shape::RoundCylinder {
            half_height,
            radius,
            border_radius,
        } => rt::ColliderGeometry::RoundCylinder {
            half_height,
            radius,
            border_radius,
        },
        Shape::ConvexHull { points } => rt::ColliderGeometry::ConvexHull(points),
        Shape::ConvexDecomposition { vertices, indices } => {
            molla_geometry::decomposition::convex_decomposition(
                &vertices,
                &indices,
                Default::default(),
            )?
        }
        Shape::TriMesh { vertices, indices } => {
            rt::ColliderGeometry::TriMesh(sim::TriMesh { vertices, indices })
        }
        Shape::Heightfield {
            rows,
            cols,
            heights,
            scale,
        } => rt::ColliderGeometry::Heightfield(sim::Heightfield::from_scaled(
            rows, cols, heights, scale,
        )?),
    })
}

pub(super) fn shape_view(shape: molla_geometry::collider_query::ColliderShapeView) -> ShapeView {
    use molla_geometry::collider_query::ColliderShapeView as View;
    match shape {
        View::Cuboid { half_extents } => ShapeView::Cuboid { half_extents },
        View::Ball { radius } => ShapeView::Ball { radius },
        View::Capsule { a, b, radius } => ShapeView::Capsule { a, b, radius },
        View::Cylinder {
            half_height,
            radius,
        } => ShapeView::Cylinder {
            half_height,
            radius,
        },
        View::RoundCylinder {
            half_height,
            radius,
            border_radius,
        } => ShapeView::RoundCylinder {
            half_height,
            radius,
            border_radius,
        },
        View::ConvexPolyhedron { points, edges } => ShapeView::ConvexPolyhedron { points, edges },
        View::Other => ShapeView::Other,
    }
}
