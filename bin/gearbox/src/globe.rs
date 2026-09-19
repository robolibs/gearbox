//! The planet's own frame and the sites on it.
//!
//! The root grid is fixed to the planet, its origin at the planet's centre.
//! A site is a grid tangent to the surface with its own Y up: machines, ground
//! and grass live in a site's coordinates, so nothing in a site knows it is on
//! a globe. The camera is the floating origin inside the site it looks at, and
//! the planet and every other site are placed around it in high precision.

use bevy::math::DVec3;
use bevy::prelude::*;
use big_space::prelude::*;

pub const PLANET_RADIUS_M: f64 = 6_371_000.0;
/// Edge of the planet frame's cells.
const PLANET_CELL_M: f32 = 10_000.0;
/// A site is one cell, wide enough to hold the view out to orbit: inside it
/// the view's frame and the site's are the same.
const SITE_CELL_M: f32 = 1.0e9;

pub struct GlobePlugin;

impl Plugin for GlobePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(BigSpaceDefaultPlugins)
            .add_systems(PreStartup, spawn_globe);
    }
}

/// The planet frame and the site everything starts on.
#[derive(Resource, Clone, Copy)]
pub struct Globe {
    pub root: Entity,
    pub home: Entity,
}

/// A tangent frame on the planet: `up` points from the planet's centre to it.
#[derive(Component, Clone, Debug)]
pub struct Site {
    pub name: String,
    pub up: DVec3,
}

fn spawn_globe(mut commands: Commands) {
    let grid = Grid::new(PLANET_CELL_M, 0.0);
    let (cell, local) = grid.translation_to_grid(DVec3::Y * PLANET_RADIUS_M);
    let root = commands
        .spawn((Name::new("Planet frame"), BigSpaceRootBundle { grid, ..default() }))
        .id();
    let home = commands
        .spawn((
            Name::new("Site home"),
            Site { name: "home".into(), up: DVec3::Y },
            BigGridBundle {
                transform: Transform::from_translation(local),
                cell,
                grid: Grid::new(SITE_CELL_M, 0.0),
                ..default()
            },
            ChildOf(root),
        ))
        .id();
    commands.insert_resource(Globe { root, home });
}
