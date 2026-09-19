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
use gearbox_globe::{Datum, Geodetic, Terrain};

/// Edge of the planet frame's cells.
const PLANET_CELL_M: f32 = 10_000.0;
/// A site is one cell, wide enough to hold the view out to orbit: inside it
/// the view's frame and the site's are the same.
const SITE_CELL_M: f32 = 1.0e9;

pub struct GlobePlugin;

impl Plugin for GlobePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(BigSpaceDefaultPlugins)
            .init_resource::<Goto>()
            .add_systems(PreStartup, spawn_globe)
            .add_systems(PreUpdate, adopt_loaded_roots)
            .add_systems(
                Update,
                (join_watched_site, travel, orient_sky, seat_planet)
                    .chain()
                    .before(crate::terrain::TerrainUpdates),
            );
    }
}

/// Marks a site's grid; `0` indexes [`Sites`].
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Site(pub usize);

#[derive(Clone, Debug)]
pub struct SiteEntry {
    pub entity: Entity,
    pub name: String,
    pub frame: Datum,
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
pub fn site_bundle(index: usize, name: &str, frame: &Datum, root: Entity) -> impl Bundle {
    let (cell, local) = Grid::new(PLANET_CELL_M, 0.0).translation_to_grid(frame.origin);
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

/// Where on Earth the world starts: `GEARBOX_DATUM="lat,lon"` in degrees, else
/// the field outside Amsterdam the GNSS has always reported.
fn home_datum() -> Datum {
    let given = std::env::var("GEARBOX_DATUM").ok().and_then(|value| {
        let (lat, lon) = value.split_once(',')?;
        Some((lat.trim().parse::<f64>().ok()?, lon.trim().parse::<f64>().ok()?))
    });
    let (latitude, longitude) = given.unwrap_or((52.370216, 4.895168));
    Datum::at(latitude, longitude)
}

fn spawn_globe(mut commands: Commands) {
    let root = commands
        .spawn((
            Name::new("Planet frame"),
            BigSpaceRootBundle { grid: Grid::new(PLANET_CELL_M, 0.0), ..default() },
        ))
        .id();
    let frame = home_datum();
    let entity = commands.spawn(site_bundle(0, "home", &frame, root)).id();
    let sites = Sites {
        root,
        list: vec![SiteEntry { entity, name: "home".into(), frame }],
        current: 0,
        land: Terrain::new(&frame),
    };
    publish_frames(&sites);
    commands.insert_resource(sites);
}

static CURRENT_SITE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The site the view is in, for code with no access to [`Sites`].
pub fn current_site() -> usize {
    CURRENT_SITE.load(std::sync::atomic::Ordering::Relaxed)
}

pub(crate) fn set_current_site(index: usize) {
    CURRENT_SITE.store(index, std::sync::atomic::Ordering::Relaxed);
}

/// A physics position as its site sees it: the site, and the place in it.
pub fn site_local(x: f64, y: f64, z: f64) -> (usize, [f64; 3]) {
    let site = gearbox_globe::region_of_physics(x);
    (site, [x - gearbox_globe::physics_offset(site).x, y, z])
}

/// The site an entity stands in: the nearest [`Site`] above it, home if none.
pub fn site_of(entity: Entity, parents: &Query<&ChildOf>, sites: &Query<&Site>) -> usize {
    std::iter::once(entity)
        .chain(parents.iter_ancestors(entity))
        .find_map(|e| sites.get(e).ok())
        .map_or(0, |site| site.0)
}

/// An entity's transform in its site's frame: its chain of local transforms
/// up to, and not including, the site's grid.
pub fn transform_in_site(
    entity: Entity,
    parents: &Query<&ChildOf>,
    transforms: &Query<&Transform>,
    sites: &Query<&Site>,
) -> GlobalTransform {
    let mut chain = GlobalTransform::from(transforms.get(entity).copied().unwrap_or_default());
    for ancestor in parents.iter_ancestors(entity) {
        if sites.contains(ancestor) {
            break;
        }
        chain = GlobalTransform::from(transforms.get(ancestor).copied().unwrap_or_default()) * chain;
    }
    chain
}

/// What is loaded stands where the view is: a loaded root joins the site in view.
fn adopt_loaded_roots(
    mut commands: Commands,
    sites: Res<Sites>,
    roots: Query<Entity, (Added<crate::load::LoadedAsset>, Without<ChildOf>)>,
) {
    for root in &roots {
        commands.entity(root).insert(ChildOf(sites.current().entity));
    }
}

/// A site is left for another once the view rests this far from where it
/// touches the planet, and an existing site is taken if it touches this near.
const LEAVE_SITE_M: f64 = 0.8 * gearbox_globe::DATUM_REACH_M;
const JOIN_SITE_M: f64 = 0.6 * gearbox_globe::DATUM_REACH_M;

impl Sites {
    fn nearest(&self, up: DVec3) -> Option<usize> {
        // `up` is an ECEF place on the ground.

        (0..self.list.len())
            .map(|index| (index, self.list[index].frame.ground_distance(up)))
            .filter(|(_, distance)| *distance < JOIN_SITE_M)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }
}

/// A place the view is asked to go: latitude and longitude in degrees.
#[derive(Resource, Default)]
pub struct Goto(pub Option<(f64, f64)>);

/// `GEARBOX_CAMERA_LLA="lat,lon"`: where a scripted view starts.
fn scripted_site() -> Option<(f64, f64)> {
    let value = std::env::var("GEARBOX_CAMERA_LLA").ok()?;
    let (bearing, distance) = value.split_once(',')?;
    Some((bearing.trim().parse().ok()?, distance.trim().parse().ok()?))
}

/// The view moves over the planet from site to site. Where it rests too far
/// from its site it joins the site already there, or takes its own site along
/// if nothing stands in it, or founds a new one. Its focus and heading carry
/// over through the planet frame, so the view itself does not jump.
fn travel(
    mut commands: Commands,
    mut sites: ResMut<Sites>,
    physics: Res<crate::physics::PhysicsWorld>,
    mut cameras: Query<(Entity, &mut mara::ui::modules::bevy::ChaseCamera)>,
    mut grids: Query<(&mut Transform, &mut CellCoord), With<Site>>,
    layout: Res<gearbox_fields::FieldLayout>,
    follow: Res<crate::viewer::state::FollowTarget>,
    fly: Res<crate::viewer::state::ChaseCameraFly>,
    mut goto: ResMut<Goto>,
    mut started: Local<bool>,
) {
    let Ok((camera, mut chase)) = cameras.single_mut() else {
        return;
    };
    let from = sites.current().frame;
    let asked = goto.0.take().or_else(|| if *started { None } else { scripted_site() });
    *started = true;
    let scripted = asked.map(|(bearing, distance)| {
        // The pair is latitude and longitude.
        Geodetic::new(bearing, distance, 0.0).ecef()
    });
    let focus = chase.focus.as_dvec3();
    // A view held on a machine stays in the machine's site.
    let held = follow.entity.is_some() || fly.target.is_some();
    let destination = match scripted {
        Some(up) => up,
        None if !held && focus.x.hypot(focus.z) > LEAVE_SITE_M => {
            let under = from.geodetic(DVec3::new(focus.x, 0.0, focus.z));
            Geodetic::new(under.latitude, under.longitude, 0.0).ecef()
        }
        None => return,
    };
    let here = sites.current;
    let occupied = physics.bodies.iter().any(|(_, body)| {
        body.is_dynamic() && gearbox_globe::region_of_physics(body.translation().x) == here
    });
    let frame = Datum::under(destination);
    let to = if let Some(index) = sites.nearest(destination).filter(|index| *index != here) {
        index
    } else if here != 0 && !occupied {
        let entry = &mut sites.list[here];
        entry.frame = frame;
        if let Ok((mut transform, mut cell)) = grids.get_mut(entry.entity) {
            let (new_cell, local) = Grid::new(PLANET_CELL_M, 0.0).translation_to_grid(frame.origin);
            *cell = new_cell;
            *transform = Transform::from_translation(local).with_rotation(frame.rotation.as_quat());
        }
        here
    } else {
        let index = sites.list.len();
        let name = format!("site-{index}");
        let entity = commands.spawn(site_bundle(index, &name, &frame, sites.root)).id();
        sites.list.push(SiteEntry { entity, name, frame });
        index
    };
    let into = sites.list[to].frame;
    // The focus lands on the ground of the new site; a scripted view starts at its middle.
    let carried = into.from_ecef(destination);
    let heading = from.rotation * DVec3::new(chase.yaw.sin() as f64, 0.0, chase.yaw.cos() as f64);
    let heading = into.rotation.inverse() * heading;
    // It keeps its height above the ground, not its height.
    let above = chase.focus.y - sites.height(here, chase.focus.x, chase.focus.z);
    let (x, z) = if scripted.is_some() { (0.0, 0.0) } else { (carried.x as f32, carried.z as f32) };
    chase.focus = Vec3::new(x, sites.height(to, x, z) + above, z);
    chase.yaw = heading.x.atan2(heading.z) as f32;
    enter_site(&mut commands, &mut sites, &layout, camera, to);
}

static FRAMES: std::sync::RwLock<Vec<Datum>> = std::sync::RwLock::new(Vec::new());
static LAND: std::sync::OnceLock<Terrain> = std::sync::OnceLock::new();

fn publish_frames(sites: &Sites) {
    let _ = LAND.set(sites.land);
    if let Ok(mut frames) = FRAMES.write() {
        *frames = sites.list.iter().map(|site| site.frame).collect();
    }
}

/// Height of the ground under a place in the physics world, in its own site's
/// frame: the drawn ground where the view is, the land itself anywhere else.
pub fn ground_height_at_physics(x: f64, z: f64) -> f64 {
    let (site, local) = site_local(x, 0.0, z);
    if site == current_site() {
        return crate::world::terrain_height_m(local[0] as f32, local[2] as f32) as f64;
    }
    let frame = FRAMES.read().ok().and_then(|frames| frames.get(site).copied());
    match (frame, LAND.get()) {
        (Some(frame), Some(land)) => land.local_height(&frame, local[0], local[2]) as f64,
        _ => 0.0,
    }
}

/// The fields laid out at home; every other site is open land of the same cover.
#[derive(Resource, Clone)]
struct HomeLayout(gearbox_fields::FieldLayout);

fn enter_site(
    commands: &mut Commands,
    sites: &mut Sites,
    layout: &gearbox_fields::FieldLayout,
    camera: Entity,
    to: usize,
) {
    if sites.current == 0 {
        commands.insert_resource(HomeLayout(layout.clone()));
    }
    sites.current = to;
    set_current_site(to);
    publish_frames(sites);
    commands.entity(camera).insert((ChildOf(sites.list[to].entity), CellCoord::default()));
    commands.queue(move |world: &mut World| {
        let Some(home) = world.get_resource::<HomeLayout>().cloned() else {
            return;
        };
        let fields = if to == 0 { home.0.fields.clone() } else { Vec::new() };
        world.insert_resource(gearbox_fields::FieldLayout { default: home.0.default, fields });
    });
    info!("globe: view now in site `{}` ({} sites)", sites.list[to].name, sites.list.len());
}

/// Flying to a machine, or following one, takes the view to the machine's site.
fn join_watched_site(
    mut commands: Commands,
    mut sites: ResMut<Sites>,
    layout: Res<gearbox_fields::FieldLayout>,
    follow: Res<crate::viewer::state::FollowTarget>,
    mut fly: ResMut<crate::viewer::state::ChaseCameraFly>,
    mut cameras: Query<(Entity, &mut mara::ui::modules::bevy::ChaseCamera)>,
    parents: Query<&ChildOf>,
    transforms: Query<&Transform>,
    grids: Query<&Site>,
) {
    let Some(watched) = fly.target.as_ref().map(|target| target.body).or(follow.entity) else {
        return;
    };
    let to = site_of(watched, &parents, &grids);
    let Ok((camera, mut chase)) = cameras.single_mut() else {
        return;
    };
    if to == sites.current || to >= sites.list.len() {
        return;
    }
    // Across the planet there is no path worth flying: the view cuts over.
    fly.target = None;
    chase.focus = transform_in_site(watched, &parents, &transforms, &grids).translation();
    enter_site(&mut commands, &mut sites, &layout, camera, to);
}

/// The sun and the weather are the planet's: the sky of a site is theirs as
/// seen from where it stands.
fn orient_sky(sites: Res<Sites>, mut weather: ResMut<bevy_weather::WeatherSettings>) {
    // The sun is set for home's sky, so home's frame stands for the planet's.
    let rotation = (sites.home().frame.rotation.inverse() * sites.current().frame.rotation).as_quat();
    if weather.planet_from_site != rotation {
        weather.planet_from_site = rotation;
    }
}

/// The sphere drawn for the planet.
#[derive(Component)]
pub struct PlanetBall;

/// The Earth is an ellipsoid and the drawn planet a sphere, so the sphere is
/// seated to touch the ground under the datum in view, where it is looked at.
fn seat_planet(
    sites: Res<Sites>,
    mut balls: Query<(&mut Transform, &mut CellCoord), With<PlanetBall>>,
    mut seated: Local<Option<DVec3>>,
) {
    let datum = sites.current().frame;
    if *seated == Some(datum.origin) {
        return;
    }
    let centre = datum.origin - datum.up() * gearbox_globe::PLANET_RADIUS_M;
    for (mut transform, mut cell) in &mut balls {
        let (new_cell, local) = Grid::new(PLANET_CELL_M, 0.0).translation_to_grid(centre);
        *cell = new_cell;
        transform.translation = local;
        transform.rotation = datum.rotation.as_quat();
        *seated = Some(datum.origin);
    }
}
