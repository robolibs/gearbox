use super::{machine::FemMachineLayout, rigid_machine::FemRigidMachine};
use crate::controller::MachineInstanceSpec;
use bevy::prelude::{Entity, World};
use molla_core::{Error, Result, WorldId};
use molla_math::Vec3;
use molla_sim::soft_belt::path::BeltPath;
use molla_sim::soft_belt::{RubberBeltMesh, RubberBeltParams, RubberCordParams, RubberLugParams};
use molla_sim::{Model, ModelBuilder, State, TetViscosity, compute_tet_surface_triangles};

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct TrackFemSpec {
    pub version: u32,
    pub path_reference: String,
    pub calibration: String,
    pub width_stations: Vec<f64>,
    pub thickness: f64,
    pub thickness_cells: usize,
    pub segments_per_pitch: usize,
    pub sprocket_teeth: usize,
    pub density: f64,
    pub youngs_modulus: f64,
    pub poisson_ratio: f64,
    /// Total Green-strain shear viscosity (Pa s).
    #[serde(default)]
    pub shear_viscosity: f64,
    /// Total Green-strain bulk viscosity (Pa s).
    #[serde(default)]
    pub bulk_viscosity: f64,
    pub cord_axial_rigidity: f64,
    pub cord_linear_density: f64,
    pub cord_prestrain: f64,
    pub guide_rows: Vec<GuideRow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outer_rows: Vec<GuideRow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outer_phase_offsets: Vec<f64>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct GuideRow {
    pub segment_span: usize,
    pub width_start: usize,
    pub width_cells: usize,
    pub depth: f64,
    pub depth_cells: usize,
}

pub(crate) struct PreparedTrackMesh {
    pub carrier: Entity,
    pub nodes: Vec<u32>,
    pub surface: Vec<[u32; 3]>,
    pub neutral_length: f64,
    pub material_pitch: f64,
    pub pitch_radius_difference: f64,
    pub mass: f64,
    pub minimum_j: f64,
}

/// Prepared soft meshes; rigid mass partition and runtime ownership remain separate.
pub(crate) struct FemTrackMeshes {
    pub model: Model,
    pub state: State,
    pub tracks: Vec<PreparedTrackMesh>,
}

fn invalid(message: &str) -> Error {
    Error::Build(format!("FEM track: {message}"))
}

#[test]
fn fem_geometry_json_preserves_exact_phase_offsets() {
    let values = [-0.009265739383448171_f64, 0.0037342606165518266];
    let encoded = serde_json::to_string(&values).unwrap();
    let decoded: Vec<f64> = serde_json::from_str(&encoded).unwrap();
    assert_eq!(
        values.map(f64::to_bits).as_slice(),
        decoded.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
}

#[test]
fn fem_material_json_defaults_viscosity_to_zero_and_round_trips() {
    let legacy = serde_json::json!({
        "version":1, "path_reference":"outer_surface", "calibration":"estimated",
        "width_stations":[-0.09,0.09], "thickness":0.021, "thickness_cells":2,
        "segments_per_pitch":4, "sprocket_teeth":14, "density":1100.0,
        "youngs_modulus":2e6, "poisson_ratio":0.45, "cord_axial_rigidity":2e5,
        "cord_linear_density":0.1, "cord_prestrain":0.005, "guide_rows":[],
    });
    let mut parsed: TrackFemSpec = serde_json::from_value(legacy).unwrap();
    assert_eq!([parsed.shear_viscosity, parsed.bulk_viscosity], [0.0; 2]);
    parsed.shear_viscosity = 1379.3103448275863;
    parsed.bulk_viscosity = 13333.333333333336;
    let round_trip: TrackFemSpec = serde_json::from_str(&serde_json::to_string(&parsed).unwrap()).unwrap();
    assert_eq!(parsed.shear_viscosity.to_bits(), round_trip.shear_viscosity.to_bits());
    assert_eq!(parsed.bulk_viscosity.to_bits(), round_trip.bulk_viscosity.to_bits());
}

impl FemTrackMeshes {
    pub(crate) fn prepare(
        world: &World,
        spec: &MachineInstanceSpec,
        layout: &FemMachineLayout,
        rigid: &FemRigidMachine,
    ) -> Result<Self> {
        if spec.tracks.len() != layout.tracks.len() {
            return Err(invalid("track layout does not match specification"));
        }
        let mut builder = ModelBuilder::new();
        builder.set_world_gravity(WorldId(0), -Vec3::Y * 9.81);
        builder.set_linear_damping(0.0);
        let mut tracks = Vec::new();
        let mut installations = Vec::new();
        for (authored, binding) in spec.tracks.iter().zip(&layout.tracks) {
            if world
                .get::<usd_bevy::UsdPrimRef>(binding.carrier)
                .is_none_or(|prim| prim.path != authored.carrier)
            {
                return Err(invalid("carrier does not match authored track order"));
            }
            let p = authored
                .fem
                .as_ref()
                .ok_or_else(|| invalid("missing explicit FEM material and geometry"))?;
            let viscosity = TetViscosity { shear: p.shear_viscosity, bulk: p.bulk_viscosity };
            viscosity.validate()?;
            if !matches!(p.version, 1 | 2)
                || !matches!(p.calibration.as_str(), "estimated" | "measured")
                || p.path_reference != "outer_surface"
                || p.width_stations.len() < 2
                || p.sprocket_teeth < 3
                || p.segments_per_pitch < 2
                || p.thickness_cells == 0
                || p.thickness_cells % 2 != 0
                || p.guide_rows.is_empty()
                || (p.version == 1
                    && (!p.outer_rows.is_empty() || !p.outer_phase_offsets.is_empty()))
                || (p.version == 2 && (p.outer_rows.is_empty() || p.outer_phase_offsets.is_empty()))
            {
                return Err(invalid(
                    "unsupported geometry version, reference or resolution",
                ));
            }
            let points = authored
                .path
                .iter()
                .map(|p| Vec3::new(p[0] as f64, p[1] as f64, p[2] as f64))
                .collect::<Vec<_>>();
            let path = BeltPath::from_closed_polyline(&points, p.thickness * 0.5)?;
            let segments = authored
                .treads
                .len()
                .checked_mul(p.segments_per_pitch)
                .ok_or_else(|| invalid("segment overflow"))?;
            let params = RubberBeltParams {
                radius: path.length() / std::f64::consts::TAU,
                width: p.width_stations.last().unwrap() - p.width_stations[0],
                thickness: p.thickness,
                segments,
                width_cells: p.width_stations.len() - 1,
                thickness_cells: p.thickness_cells,
                density: p.density,
                youngs_modulus: p.youngs_modulus,
                poisson_ratio: p.poisson_ratio,
            };
            let lug_rows = |rows: &[GuideRow]| {
                rows.iter()
                    .map(|g| RubberLugParams {
                        count: authored.treads.len(),
                        segment_span: g.segment_span,
                        width_start: g.width_start,
                        width_cells: g.width_cells,
                        depth: g.depth,
                        depth_cells: g.depth_cells,
                    })
                    .collect::<Vec<_>>()
            };
            let mut mesh =
                RubberBeltMesh::circular(params)?.with_width_stations(&p.width_stations)?;
            if p.version == 2 {
                mesh =
                    mesh.with_outer_phase_offsets(authored.treads.len(), &p.outer_phase_offsets)?;
            }
            let mesh =
                mesh.with_surface_lug_rows(&lug_rows(&p.guide_rows), &lug_rows(&p.outer_rows))?;
            let phase =
                -0.5 * p.guide_rows[0].segment_span as f64 * path.length() / segments as f64;
            if p.guide_rows
                .iter()
                .any(|g| g.segment_span != p.guide_rows[0].segment_span)
            {
                return Err(invalid("guide rows require a shared phase"));
            }
            let installed = mesh.installed_positions(&path, phase, Vec3::ZERO)?;
            let minimum_j = mesh.minimum_volume_ratio(&installed)?;
            if minimum_j <= 0.5 {
                return Err(invalid("installed belt violates volume gate"));
            }
            let body = *rigid
                .bodies
                .get(&binding.carrier)
                .ok_or_else(|| invalid("carrier is missing from rigid model"))?;
            let pose = rigid.state.body_q.host()?[body.index()];
            let installed = installed
                .into_iter()
                .map(|p| pose.transform_point(p))
                .collect::<Vec<_>>();
            let handles = mesh.append_reinforced_to(
                &mut builder,
                WorldId(0),
                Vec3::ZERO,
                RubberCordParams {
                    axial_rigidity: p.cord_axial_rigidity,
                    linear_density: p.cord_linear_density,
                    prestrain: p.cord_prestrain,
                },
            )?;
            builder.set_tet_mesh_viscosity(handles.mesh, viscosity)?;
            let surface = compute_tet_surface_triangles(&mesh.tets)
                .into_iter()
                .map(|tri| tri.map(|n| handles.nodes[n as usize]))
                .collect();
            let material_pitch = path.length() / authored.treads.len() as f64;
            tracks.push(PreparedTrackMesh {
                carrier: binding.carrier,
                nodes: handles.nodes,
                surface,
                neutral_length: path.length(),
                material_pitch,
                pitch_radius_difference: authored.radius
                    - material_pitch * p.sprocket_teeth as f64 / std::f64::consts::TAU,
                mass: 0.0,
                minimum_j,
            });
            installations.push(installed);
        }
        let model = builder.build()?;
        let mut state = model.state()?;
        for (track, installed) in tracks.iter_mut().zip(installations) {
            for (&node, point) in track.nodes.iter().zip(installed) {
                state.particle_q.host_mut()?[node as usize] = point;
            }
            track.mass = track
                .nodes
                .iter()
                .map(|&node| model.particle_mass.host().map(|m| m[node as usize]))
                .sum::<Result<f64>>()?;
        }
        Ok(Self {
            model,
            state,
            tracks,
        })
    }
}
