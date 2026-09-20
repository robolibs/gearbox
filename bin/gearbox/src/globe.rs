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
                (reanchor_machines, join_watched_site, travel, orient_sky, seat_planet)
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
    /// Frames the view left with nothing in them, oldest first, to be used again.
    pub spare: Vec<usize>,
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
        spare: Vec::new(),
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

/// A loaded root joins the datum it was placed in, or the one in view if it was
/// placed nowhere in particular.
fn adopt_loaded_roots(
    mut commands: Commands,
    sites: Res<Sites>,
    roots: Query<(Entity, Option<&InDatum>), (Added<crate::load::LoadedAsset>, Without<ChildOf>)>,
) {
    for (root, placed) in &roots {
        let datum = placed.map_or(sites.current, |placed| placed.0.min(sites.list.len() - 1));
        commands.entity(root).insert(ChildOf(sites.list[datum].entity));
    }
}

/// A site is left for another once the view rests this far from where it
/// touches the planet, and an existing site is taken if it touches this near.
/// `GEARBOX_DATUM_REACH_M` shrinks a datum's reach, to watch re-anchoring happen.
fn reach() -> f64 {
    static REACH: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *REACH.get_or_init(|| {
        std::env::var("GEARBOX_DATUM_REACH_M")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| *value > 100.0)
            .unwrap_or(gearbox_globe::DATUM_REACH_M)
    })
}

impl Sites {
    fn nearest(&self, up: DVec3) -> Option<usize> {
        // `up` is an ECEF place on the ground.

        (0..self.list.len())
            .map(|index| (index, self.list[index].frame.ground_distance(up)))
            .filter(|(_, distance)| *distance < (0.6 * reach()))
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
        None if !held && focus.x.hypot(focus.z) > (0.8 * reach()) => {
            let under = from.geodetic(DVec3::new(focus.x, 0.0, focus.z));
            Geodetic::new(under.latitude, under.longitude, 0.0).ecef()
        }
        None => return,
    };
    let here = sites.current;
    let occupied = physics.bodies.iter().any(|(_, body)| {
        body.is_dynamic() && gearbox_globe::region_of_physics(body.translation().x) == here
    });
    let to = sites.datum_for(&mut commands, destination);
    if to == here {
        return;
    }
    if here != 0 && !occupied && !sites.spare.contains(&here) {
        sites.spare.push(here);
    }
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
        // Only the home site is laid out; elsewhere the default cover stands
        // alone, so its roads have nothing to cross either.
        let (fields, ways) = if to == 0 {
            (home.0.fields.clone(), home.0.ways.clone())
        } else {
            (Vec::new(), Vec::new())
        };
        world.insert_resource(gearbox_fields::FieldLayout {
            default: home.0.default,
            fields,
            ways,
        });
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

/// Where on Earth a place in datum `region`'s frame is: its latitude,
/// longitude and altitude, the same in ECEF, and the datum's own anchor.
pub struct EarthPlace {
    pub geodetic: Geodetic,
    pub ecef: DVec3,
    pub datum: Datum,
}

pub fn earth_place(region: usize, local: [f64; 3]) -> Option<EarthPlace> {
    let datum = FRAMES.read().ok()?.get(region).copied()?;
    let ecef = datum.to_ecef(DVec3::from_array(local));
    Some(EarthPlace { geodetic: Geodetic::of_ecef(ecef), ecef, datum })
}

/// Put on a loaded root: the datum it was placed in.
#[derive(Component, Clone, Copy)]
pub struct InDatum(pub usize);

impl Sites {
    /// The datum that serves an ECEF place on the ground: the nearest one in
    /// reach, else a new one anchored there.
    pub fn datum_for(&mut self, commands: &mut Commands, ground: DVec3) -> usize {
        if let Some(index) = self.nearest(ground) {
            self.spare.retain(|spare| *spare != index);
            return index;
        }
        let frame = Datum::under(ground);
        // The oldest spare frame is moved here, once a newer spare shows that
        // nothing of it is still being drawn.
        if self.spare.len() >= 2 {
            let index = self.spare.remove(0);
            let (cell, local) = Grid::new(PLANET_CELL_M, 0.0).translation_to_grid(frame.origin);
            commands.entity(self.list[index].entity).insert((
                cell,
                Transform::from_translation(local).with_rotation(frame.rotation.as_quat()),
            ));
            self.list[index].frame = frame;
            publish_frames(self);
            return index;
        }
        let index = self.list.len();
        let name = format!("datum-{index}");
        let entity = commands.spawn(site_bundle(index, &name, &frame, self.root)).id();
        self.list.push(SiteEntry { entity, name, frame });
        publish_frames(self);
        index
    }

    /// Where a request lands: `lla` if given, else `at` as metres north, up
    /// and east of home. Returns the datum and the place in its frame; a
    /// height of zero means on the ground.
    pub fn place(
        &mut self,
        commands: &mut Commands,
        lla: Option<Geodetic>,
        at: Vec3,
    ) -> (usize, Vec3, Geodetic) {
        let asked = lla.unwrap_or_else(|| {
            let mut there = self.home().frame.geodetic(at.as_dvec3());
            there.altitude = at.y as f64;
            there
        });
        let ground = Geodetic::new(asked.latitude, asked.longitude, 0.0).ecef();
        let region = self.datum_for(commands, ground);
        let local = self.list[region].frame.from_ecef(ground);
        let (x, z) = (local.x as f32, local.z as f32);
        let y = if asked.altitude.abs() < 0.001 { self.height(region, x, z) } else { asked.altitude as f32 };
        (region, Vec3::new(x, y, z), asked)
    }
}

/// A machine that has gone far from its datum is given a new one where it is,
/// so it can go on forever: its bodies, and whatever is hitched to it, are
/// re-expressed in the new datum through ECEF, velocities and all. Nothing
/// about where it is on Earth changes.
fn reanchor_machines(
    mut commands: Commands,
    mut sites: ResMut<Sites>,
    mut physics: ResMut<crate::physics::PhysicsWorld>,
    parents: Query<&ChildOf>,
    loaded: Query<(), With<crate::load::LoadedAsset>>,
    mut transforms: Query<&mut Transform>,
) {
    use rapier3d::math::{Rotation, Vector};
    const TOGETHER_M: f64 = 1_000.0;
    let strayed = physics.bodies.iter().find_map(|(_, body)| {
        let at = body.translation();
        let (region, local) = site_local(at.x, at.y, at.z);
        (body.is_dynamic() && region < sites.list.len() && local[0].hypot(local[2]) > 0.8 * reach())
            .then_some((region, DVec3::from_array(local)))
    });
    let Some((from_region, trigger)) = strayed else {
        return;
    };
    let from = sites.list[from_region].frame;
    let under = from.geodetic(trigger);
    let ground = Geodetic::new(under.latitude, under.longitude, 0.0).ecef();
    let to_region = sites.datum_for(&mut commands, ground);
    if to_region == from_region {
        return;
    }
    let into = sites.list[to_region].frame;
    let turn = into.rotation.inverse() * from.rotation;
    let (from_x, to_x) = (
        gearbox_globe::physics_offset(from_region).x,
        gearbox_globe::physics_offset(to_region).x,
    );
    let physics = physics.as_mut();
    let moved: Vec<Entity> = physics
        .entity_to_body
        .iter()
        .filter(|(_, handle)| {
            physics.bodies.get(**handle).is_some_and(|body| {
                let at = body.translation();
                let (region, local) = site_local(at.x, at.y, at.z);
                region == from_region && (DVec3::from_array(local) - trigger).length() < TOGETHER_M
            })
        })
        .map(|(entity, _)| *entity)
        .collect();
    let spin = |v: Vector| {
        let turned = turn * DVec3::new(v.x, v.y, v.z);
        Vector::new(turned.x, turned.y, turned.z)
    };
    for entity in &moved {
        let Some(body) = physics.entity_to_body.get(entity).and_then(|h| physics.bodies.get_mut(*h)) else {
            continue;
        };
        let mut pose = *body.position();
        let local = DVec3::new(pose.translation.x - from_x, pose.translation.y, pose.translation.z);
        let carried = into.from_ecef(from.to_ecef(local));
        pose.translation = Vector::new(carried.x + to_x, carried.y, carried.z);
        let q = pose.rotation;
        let turned = turn * bevy::math::DQuat::from_xyzw(q.x, q.y, q.z, q.w);
        pose.rotation = Rotation::from_xyzw(turned.x, turned.y, turned.z, turned.w);
        let (linvel, angvel) = (spin(body.linvel()), spin(body.angvel()));
        body.set_position(pose, true);
        body.set_linvel(linvel, true);
        body.set_angvel(angvel, true);
    }
    physics.bodies.propagate_modified_body_positions_to_colliders(&mut physics.colliders);
    // The roots the bodies hang from change datum with them.
    let mut roots: Vec<Entity> = moved
        .iter()
        .filter_map(|entity| {
            std::iter::once(*entity).chain(parents.iter_ancestors(*entity)).find(|e| loaded.contains(*e))
        })
        .collect();
    roots.sort();
    roots.dedup();
    for root in roots {
        if let Ok(mut transform) = transforms.get_mut(root) {
            let carried = into.from_ecef(from.to_ecef(transform.translation.as_dvec3()));
            transform.translation = carried.as_vec3();
            transform.rotation = turn.as_quat() * transform.rotation;
        }
        commands.entity(root).insert((ChildOf(sites.list[to_region].entity), InDatum(to_region)));
    }
    info!(
        "globe: {} bodies re-anchored to datum {:.5}, {:.5}",
        moved.len(),
        into.latitude,
        into.longitude
    );
}

static MACHINE_DATUMS: std::sync::RwLock<Option<std::collections::HashMap<String, Datum>>> =
    std::sync::RwLock::new(None);

/// A machine's own datum: the fixed anchor its reported x, y, z are measured
/// from, north, up and east. It is where the machine was put down unless the
/// request named another, and it never moves; the frames the simulation
/// works in are a separate matter and never show.
pub fn set_machine_datum(machine: &str, datum: Datum) {
    if let Ok(mut datums) = MACHINE_DATUMS.write() {
        datums.get_or_insert_with(Default::default).insert(machine.to_string(), datum);
    }
}

pub fn machine_datum(machine: &str) -> Option<Datum> {
    MACHINE_DATUMS.read().ok()?.as_ref()?.get(machine).copied()
}
