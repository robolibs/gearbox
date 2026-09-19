//! The planet's own frame and the sites on it.
//!
//! The root grid is fixed to the planet, its origin at the planet's centre.
//! A site is a grid tangent to the surface with its own Y up: machines, ground
//! and grass live in a site's coordinates, so nothing in a site knows it is on
//! a globe. The camera is the floating origin inside the site it looks at, and
//! the planet and every other site are placed around it in high precision.
//! All sites share one physics world, each in its own region of it.

use bevy::math::DVec3;
use bevy::prelude::*;
use big_space::prelude::*;
use gearbox_globe::{SiteFrame, Terrain};

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

/// Marks a site's grid; `0` indexes [`Sites`].
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Site(pub usize);

#[derive(Clone, Debug)]
pub struct SiteEntry {
    pub entity: Entity,
    pub name: String,
    pub frame: SiteFrame,
}

/// Every site on the planet; the first is home and the view is in `current`.
#[derive(Resource)]
pub struct Sites {
    pub root: Entity,
    pub list: Vec<SiteEntry>,
    pub current: usize,
    pub land: Terrain,
}

impl Sites {
    pub fn home(&self) -> &SiteEntry {
        &self.list[0]
    }

    pub fn current(&self) -> &SiteEntry {
        &self.list[self.current]
    }

    /// Height of the land in site `index`'s frame.
    pub fn height(&self, index: usize, x: f32, z: f32) -> f32 {
        self.land.local_height(&self.list[index].frame, x as f64, z as f64)
    }
}

/// The components that stand a site's grid on the planet.
pub fn site_bundle(index: usize, name: &str, frame: &SiteFrame, root: Entity) -> impl Bundle {
    let (cell, local) = Grid::new(PLANET_CELL_M, 0.0).translation_to_grid(frame.origin());
    (
        Name::new(format!("Site {name}")),
        Site(index),
        BigGridBundle {
            transform: Transform::from_translation(local).with_rotation(frame.rotation.as_quat()),
            cell,
            grid: Grid::new(SITE_CELL_M, 0.0),
            ..default()
        },
        ChildOf(root),
    )
}

fn spawn_globe(mut commands: Commands) {
    let root = commands
        .spawn((
            Name::new("Planet frame"),
            BigSpaceRootBundle { grid: Grid::new(PLANET_CELL_M, 0.0), ..default() },
        ))
        .id();
    let frame = SiteFrame::at(DVec3::Y);
    let entity = commands.spawn(site_bundle(0, "home", &frame, root)).id();
    commands.insert_resource(Sites {
        root,
        list: vec![SiteEntry { entity, name: "home".into(), frame }],
        current: 0,
        land: Terrain::new(&frame),
    });
}
