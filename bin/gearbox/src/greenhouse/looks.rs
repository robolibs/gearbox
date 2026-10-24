//! What the greenhouse looks like: every crop's prefabs, the materials its
//! health tints them with, the plants standing in their trays, and the light
//! of the lamps and work lights.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI};

use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::asset::RenderAssetUsages;
use bevy::light::{GeneratedEnvironmentMapLight, NotShadowCaster};
use bevy::prelude::*;
use gearbox_greenhouse::{Edible, Form, Plant, PlantId, Species};

use super::crops::{self, STAGES};
use super::{Greenhouse, scatter};

/// How a plant's health shows on its leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tint {
    Sound,
    /// Pale from short nutrients.
    Pale,
    /// Yellow: starved, or a crop dying back as it ripens.
    Yellow,
    /// Grey-white with mould.
    Mouldy,
    /// Brown and dry.
    Dead,
}

impl Tint {
    const ALL: [Self; 5] = [Self::Sound, Self::Pale, Self::Yellow, Self::Mouldy, Self::Dead];

    fn of(plant: &Plant) -> Self {
        let ripening = matches!(plant.species.edible, Edible::Tuber | Edible::Grain) && plant.maturity() > 0.88;
        if !plant.alive {
            Self::Dead
        } else if plant.mould > 0.35 {
            Self::Mouldy
        } else if plant.nutrient_stress > 0.5 || ripening {
            Self::Yellow
        } else if plant.nutrient_stress > 0.25 || plant.water_stress > 0.4 {
            Self::Pale
        } else {
            Self::Sound
        }
    }

    /// What it multiplies a leaf photo by. The photographed leaves are a
    /// late-summer green; each crop's sound tint brings them to its own.
    fn colour(self, species: &Species) -> Color {
        let sound = match species.form {
            Form::Rosette => Vec3::new(0.78, 1.08, 0.62),
            Form::Vine => Vec3::new(0.86, 1.0, 0.80),
            Form::Bush => Vec3::new(0.84, 0.96, 0.78),
            Form::Grass => Vec3::new(0.86, 1.02, 0.66),
        };
        let v = match self {
            Self::Sound => sound,
            Self::Pale => sound.lerp(Vec3::new(1.15, 1.10, 0.55), 0.45),
            Self::Yellow => Vec3::new(1.25, 1.05, 0.30),
            Self::Mouldy => Vec3::new(0.95, 0.95, 0.88),
            Self::Dead => Vec3::new(0.62, 0.42, 0.22),
        };
        Color::linear_rgb(v.x, v.y, v.z)
    }
}

struct Meshes {
    leaves: Handle<Mesh>,
    parts: Option<Handle<Mesh>>,
    fruit: Option<Handle<Mesh>>,
}

/// Every crop's prefab meshes by stage, and materials by health.
#[derive(Resource)]
pub struct CropLooks {
    meshes: HashMap<(&'static str, usize), Meshes>,
    /// Points whose convex hull wraps each prefab.
    hulls: HashMap<(&'static str, usize), Vec<Vec3>>,
    leaves: HashMap<(&'static str, Tint), Handle<StandardMaterial>>,
    parts: Handle<StandardMaterial>,
    fruit: Handle<StandardMaterial>,
    lamp_on: HashMap<[u16; 3], Handle<StandardMaterial>>,
    lamp_off: Handle<StandardMaterial>,
}

pub fn build_looks(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let mut looks = CropLooks {
        meshes: HashMap::new(),
        hulls: HashMap::new(),
        leaves: HashMap::new(),
        parts: materials.add(StandardMaterial { perceptual_roughness: 0.7, ..default() }),
        fruit: materials.add(StandardMaterial { perceptual_roughness: 0.22, reflectance: 0.6, ..default() }),
        lamp_on: HashMap::new(),
        lamp_off: materials.add(StandardMaterial { base_color: Color::srgb(0.85, 0.85, 0.88), perceptual_roughness: 0.3, ..default() }),
    };
    for species in gearbox_greenhouse::species::ALL {
        let atlas = Image::from_buffer(
            crops::atlas(species),
            ImageType::Extension("png"),
            CompressedImageFormats::NONE,
            true,
            ImageSampler::linear(),
            RenderAssetUsages::RENDER_WORLD,
        )
        .expect("crop leaf atlas decodes");
        let atlas = images.add(atlas);
        for tint in Tint::ALL {
            let material = StandardMaterial {
                base_color: tint.colour(species),
                base_color_texture: Some(atlas.clone()),
                alpha_mode: AlphaMode::Mask(0.5),
                double_sided: true,
                cull_mode: None,
                perceptual_roughness: 0.62,
                ..default()
            };
            looks.leaves.insert((species.name, tint), materials.add(material));
        }
        for stage in 0..STAGES {
            let prefab = crops::prefab(species, stage);
            looks.hulls.insert((species.name, stage), hull_points(std::iter::once(&prefab.leaves).chain(prefab.parts.as_ref())));
            looks.meshes.insert(
                (species.name, stage),
                Meshes {
                    leaves: meshes.add(prefab.leaves),
                    parts: prefab.parts.map(|m| meshes.add(m)),
                    fruit: prefab.fruit.map(|m| meshes.add(m)),
                },
            );
        }
    }
    commands.insert_resource(looks);
}

impl CropLooks {
    /// Points whose convex hull wraps a crop's prefab at a stage, in its frame.
    pub fn hull(&self, species: &'static str, stage: usize) -> Option<&[Vec3]> {
        self.hulls.get(&(species, stage)).map(Vec::as_slice).filter(|points| points.len() >= 4)
    }
}

/// Directions a hull is sampled along.
const HULL_DIRECTIONS: usize = 96;

/// The farthest vertex of the meshes along each of a spread of directions:
/// few points, and their hull is close to the meshes' own.
fn hull_points<'a>(meshes: impl Iterator<Item = &'a Mesh>) -> Vec<Vec3> {
    let positions: Vec<Vec3> = meshes
        .filter_map(|mesh| mesh.attribute(Mesh::ATTRIBUTE_POSITION)?.as_float3())
        .flatten()
        .map(|p| Vec3::from_array(*p))
        .collect();
    let mut points: Vec<Vec3> = Vec::new();
    for i in 0..HULL_DIRECTIONS {
        // A Fibonacci sphere: even directions without a pole cluster.
        let y = 1.0 - 2.0 * (i as f32 + 0.5) / HULL_DIRECTIONS as f32;
        let turn = i as f32 * 2.399_963;
        let along = Vec3::new((1.0 - y * y).sqrt() * turn.cos(), y, (1.0 - y * y).sqrt() * turn.sin());
        let Some(far) = positions.iter().copied().max_by(|a, b| a.dot(along).total_cmp(&b.dot(along))) else { break };
        if !points.iter().any(|p| p.distance_squared(far) < 1e-8) {
            points.push(far);
        }
    }
    points
}

/// What a plant should look like now: its prefab, tint, whether it carries fruit, and its size.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct PlantLook {
    pub id: PlantId,
    pub stage: usize,
    pub tint: Tint,
    pub fruit: bool,
    pub scale: f32,
}

impl PlantLook {
    fn of(id: PlantId, plant: &Plant) -> Option<Self> {
        if !plant.emerged {
            return None;
        }
        let stage = ((plant.maturity() * (STAGES - 1) as f64).round() as usize).min(STAGES - 1);
        let alike = 0.88 + 0.24 * scatter(id.tray, id.slot, 11);
        let vigour = 0.7 + 0.3 * plant.health.max(0.25) as f32;
        Some(Self {
            id,
            stage,
            tint: Tint::of(plant),
            fruit: plant.species.edible == Edible::Fruit && plant.edible_g > 1.0,
            scale: alike * vigour,
        })
    }

    /// Looks the same as far as meshes and materials go: size changes in place.
    fn same_parts(&self, other: &Self) -> bool {
        self.stage == other.stage && self.tint == other.tint && self.fruit == other.fruit
    }
}

/// The plant entities in each tray place, and what they show.
#[derive(Resource, Default)]
pub struct PlantEntities(HashMap<PlantId, (Entity, PlantLook)>);

/// Plants stand in their trays as their stage, health and fruit say: a new
/// look replaces the entity, a new size only rescales it.
pub fn sync_plants(
    mut commands: Commands,
    greenhouse: Res<Greenhouse>,
    looks: Option<Res<CropLooks>>,
    mut shown: Local<PlantEntities>,
    mut transforms: Query<&mut Transform>,
) {
    let Some(looks) = looks else { return };
    if greenhouse.is_empty() {
        return;
    }
    let mut seen = std::collections::HashSet::new();
    for (id, plant) in greenhouse.house.plants() {
        let Some(look) = PlantLook::of(id, plant) else { continue };
        seen.insert(id);
        if let Some((entity, was)) = shown.0.get_mut(&id) {
            if was.same_parts(&look) {
                if (was.scale - look.scale).abs() > 0.01
                    && let Ok(mut transform) = transforms.get_mut(*entity)
                {
                    transform.scale = Vec3::splat(look.scale);
                    was.scale = look.scale;
                }
                continue;
            }
            commands.entity(*entity).despawn();
        }
        let place = &greenhouse.trays[id.tray];
        let at = place.slot_at(id.slot);
        let yaw = scatter(id.tray, id.slot, 5) * 2.0 * PI;
        // Prefabs stand Y-up; the tray's frame is the USD's, Z-up.
        let transform = Transform::from_xyz(at.x, at.y, 0.0)
            .with_rotation(Quat::from_rotation_x(FRAC_PI_2) * Quat::from_rotation_y(yaw))
            .with_scale(Vec3::splat(look.scale));
        let parts = &looks.meshes[&(plant.species.name, look.stage)];
        let leaves = looks.leaves[&(plant.species.name, look.tint)].clone();
        let name = format!("Plant[{}]", greenhouse.plant_name(id));
        let entity = commands
            .spawn((Name::new(name), transform, Visibility::Inherited, ChildOf(place.entity), look))
            .with_children(|plant| {
                plant.spawn((Mesh3d(parts.leaves.clone()), MeshMaterial3d(leaves), NotShadowCaster));
                if let Some(mesh) = &parts.parts {
                    plant.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(looks.parts.clone()), NotShadowCaster));
                }
                if let (Some(mesh), true) = (&parts.fruit, look.fruit) {
                    plant.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(looks.fruit.clone()), NotShadowCaster));
                }
            })
            .id();
        shown.0.insert(id, (entity, look));
    }
    shown.0.retain(|id, (entity, _)| {
        let keep = seen.contains(id);
        if !keep {
            commands.entity(*entity).despawn();
        }
        keep
    });
}

/// The lamp over a tray.
#[derive(Component)]
pub struct GrowLamp {
    pub lamp: usize,
}

/// Share of a lamp's light that falls on its own tray rather than round it.
const ON_TRAY: f32 = 0.6;

/// Mean illuminance over each room's surfaces from its lamps, lux: their
/// lumens spread over the walls, floor and ceiling of its box.
fn room_bounce(greenhouse: &Greenhouse, hour: f64) -> Vec<f32> {
    let house = &greenhouse.house;
    let mut lumens = vec![0.0f32; house.rooms.len()];
    for t in &house.trays {
        for &l in &t.lamps {
            let lamp = &house.lamps[l];
            let lux = (lamp.ppfd(hour, house.cycle_hours, house.lamp_power) * lamp.channels.lux_per_ppfd()) as f32;
            lumens[t.room] += lux * t.area_m2 as f32 / ON_TRAY;
        }
    }
    let mut out = vec![0.0f32; house.rooms.len()];
    for place in &greenhouse.rooms {
        let s = place.size;
        out[place.room] = lumens[place.room] / (2.0 * (s.x * s.y + s.y * s.z + s.x * s.z)).max(1.0);
    }
    out
}

/// Exposure a camera inside a room takes: an incident-light meter's EV100
/// for the room's light, `log2(E × 100 / 250)`, never darker than a dim room.
fn room_exposure(lux: f32) -> f32 {
    (lux.max(20.0) * 100.0 / 250.0).log2()
}

/// A camera inside a room sees by the room's light: the sky does not light
/// the inside of a closed module, so the environment light the atmosphere
/// generated for it drops to a trace, and the exposure follows the lamps
/// rather than the day outside. Only a scattered sky generates that light;
/// under another the exposure alone changes.
pub fn expose_indoors(
    greenhouse: Res<Greenhouse>,
    weather: Res<crate::environment::WeatherSettings>,
    rooms: Query<&GlobalTransform>,
    mut cameras: Query<
        (&GlobalTransform, &mut bevy::camera::Exposure, Option<&mut EnvironmentMapLight>, Has<GeneratedEnvironmentMapLight>),
        With<Camera3d>,
    >,
) {
    if greenhouse.rooms.is_empty() {
        return;
    }
    for (eye, mut exposure, environment, generated) in &mut cameras {
        let inside = greenhouse.rooms.iter().find(|place| {
            let Ok(frame) = rooms.get(place.parent) else { return false };
            let local = frame.affine().inverse().transform_point3(eye.translation()) - place.centre;
            (local.abs() * 2.0).cmple(place.size).all()
        });
        let (ev, sky) = match inside {
            Some(place) => (room_exposure(place.lux), 0.02),
            None => (weather.exposure_ev100, 1.0),
        };
        if (exposure.ev100 - ev).abs() > 0.05 {
            exposure.ev100 = ev;
        }
        // The map the atmosphere generated: one the weather paints is the weather's.
        if let Some(mut environment) = environment.filter(|_| generated)
            && (environment.intensity - sky).abs() > 1e-3
        {
            environment.intensity = sky;
        }
    }
}

/// A spot light points down from each lamp; its colour is the lamp's mix
/// and its strength the light it gives now: lux at the canopy from the
/// photon flux, over the inverse square a point source of it falls off by.
/// The lamp's face glows its colour while lit. Work lights burn while there is power.
pub fn sync_lamps(
    mut commands: Commands,
    mut greenhouse: ResMut<Greenhouse>,
    weather: Res<crate::environment::WeatherSettings>,
    looks: Option<ResMut<CropLooks>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut lights: Query<(&GrowLamp, &mut SpotLight)>,
    mut work: Query<&mut PointLight>,
    mut faces: Query<&mut MeshMaterial3d<StandardMaterial>>,
    mut spawned: Local<std::collections::HashSet<usize>>,
) {
    let Some(mut looks) = looks else { return };
    if greenhouse.is_empty() {
        return;
    }
    let cycle = greenhouse.house.cycle_hours;
    let hour = weather.calendar.hour * cycle / 24.0;
    if spawned.len() < greenhouse.trays.len() {
        for (index, place) in greenhouse.trays.iter().enumerate() {
            let Some(parent) = place.lamp else { continue };
            let lamp = greenhouse.house.trays[index].lamps[0];
            if !spawned.insert(lamp) {
                continue;
            }
            let reach = place.size.length() * 0.5;
            let angle = (reach / place.lamp_height.max(0.1)).atan().clamp(0.3, 1.3);
            commands.spawn((
                Name::new(format!("Lamp[{}]", greenhouse.house.lamps[lamp].name)),
                GrowLamp { lamp },
                SpotLight {
                    intensity: 0.0,
                    range: (place.lamp_height * 4.0).max(2.0),
                    radius: 0.15,
                    shadow_maps_enabled: false,
                    outer_angle: (angle * 1.35).min(1.45),
                    inner_angle: angle * 0.7,
                    ..default()
                },
                // Just under the panel face, so it lights the crop and not itself.
                Transform::from_xyz(0.0, 0.0, -0.012),
                ChildOf(parent),
            ));
        }
    }
    let powered = !greenhouse.house.power.cut;
    let bounce = room_bounce(&greenhouse, hour);
    let Greenhouse { rooms, .. } = &mut *greenhouse;
    for place in rooms.iter_mut() {
        // The work light hangs under the ceiling; its lux is what reaches the floor.
        let drop = place.size.z - 0.15;
        let at = place.centre + Vec3::Z * (place.size.z / 2.0 - 0.15);
        let lumens = if powered { place.work_lux * 4.0 * PI * drop * drop } else { 0.0 };
        match place.work_light.map(|e| work.get_mut(e)) {
            None if place.work_lux > 0.0 => {
                let light = PointLight { intensity: lumens, range: 8.0, shadow_maps_enabled: false, color: Color::srgb(1.0, 0.97, 0.92), ..default() };
                place.work_light = Some(commands.spawn((Name::new("Work light"), light, Transform::from_translation(at), ChildOf(place.parent))).id());
            }
            Some(Ok(mut point)) if (point.intensity - lumens).abs() > 1.0 => point.intensity = lumens,
            _ => {}
        }
        place.lux = bounce[place.room] + if powered { place.work_lux * 0.5 } else { 0.0 };
    }
    let house = &greenhouse.house;
    for (lamp, mut spot) in &mut lights {
        let Some(tray) = house.trays.iter().position(|t| t.lamps.contains(&lamp.lamp)) else { continue };
        let fixture = &house.lamps[lamp.lamp];
        let ppfd = fixture.ppfd(hour, cycle, house.lamp_power);
        let lux = (ppfd * fixture.channels.lux_per_ppfd()) as f32;
        let drop = (greenhouse.trays[tray].lamp_height * 0.85).max(0.1);
        let intensity = lux * 4.0 * PI * drop * drop;
        let [r, g, b] = fixture.channels.colour();
        let colour = Color::linear_rgb(r, g, b);
        if (spot.intensity - intensity).abs() > 0.01 * spot.intensity.max(1.0) || spot.color != colour {
            spot.intensity = intensity;
            spot.color = colour;
        }
        if let Some(face) = greenhouse.trays[tray].lamp_face
            && let Ok(mut material) = faces.get_mut(face)
        {
            let wanted = if ppfd > 0.0 {
                let key = [(r * 1000.0) as u16, (g * 1000.0) as u16, (b * 1000.0) as u16];
                looks
                    .lamp_on
                    .entry(key)
                    .or_insert_with(|| {
                        materials.add(StandardMaterial {
                            base_color: Color::linear_rgb(r, g, b),
                            emissive: LinearRgba::rgb(r, g, b) * 450.0,
                            ..default()
                        })
                    })
                    .clone()
            } else {
                looks.lamp_off.clone()
            };
            if material.0 != wanted {
                material.0 = wanted;
            }
        }
    }
}
