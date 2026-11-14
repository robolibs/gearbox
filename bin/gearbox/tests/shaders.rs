use std::path::Path;
use std::time::{Duration, Instant};

use bevy::asset::uuid::Uuid;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::settings::{RenderCreation, WgpuSettings};
use bevy::shader::{ShaderCache, ShaderCacheError, ShaderCacheSource, ShaderDefVal};

fn register_sources(
    cache: &mut ShaderCache<(), ()>,
    directory: &Path,
    prefix: &str,
    entries: &mut Vec<(String, AssetId<Shader>, bool)>,
    next_id: &mut u128,
) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let name = format!("{prefix}{}", path.file_name().unwrap().to_str().unwrap());
        if path.is_dir() {
            register_sources(cache, &path, &format!("{name}/"), entries, next_id);
        } else if path.extension().is_some_and(|ext| ext == "wesl") {
            let source = std::fs::read_to_string(&path).unwrap();
            let id = AssetId::Uuid {
                uuid: Uuid::from_u128(*next_id),
            };
            *next_id += 1;
            if source.contains("@vertex")
                || source.contains("@fragment")
                || source.contains("@compute")
            {
                entries.push((name.clone(), id, source.contains("PREPASS_PIPELINE")));
            }
            cache.set_shader(id, Shader::from_wesl(source, name));
        }
    }
}

#[test]
fn custom_shader_variants_compile_and_validate() {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: bevy::window::ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(Box::new(WgpuSettings {
                    backends: None,
                    ..default()
                })),
                ..default()
            })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>()
            .disable::<bevy::gilrs::GilrsPlugin>()
            .disable::<bevy::log::LogPlugin>(),
    );
    app.finish();
    app.cleanup();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        app.update();
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut cache = ShaderCache::new((), |_, source, _| {
        let ShaderCacheSource::Wgsl(source) = source else {
            panic!("expected WGSL")
        };
        let module = naga::front::wgsl::parse_str(&source)
            .map_err(|error| ShaderCacheError::CreateShaderModule(error.emit_to_string(&source)))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .map_err(|error| ShaderCacheError::CreateShaderModule(error.emit_to_string(&source)))?;
        Ok(())
    });
    let shaders = app.world().resource::<Assets<Shader>>();
    assert!(shaders.len() > 50, "Bevy shader libraries were not loaded");
    for (id, shader) in shaders.iter() {
        cache.set_shader(id, shader.clone());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut entries = Vec::new();
    let mut next_id = 1;
    for (directory, prefix) in [
        ("bin/gearbox/src", "embedded://gearbox/"),
        ("crates/gearbox-fields/src", "embedded://gearbox_fields/"),
        ("crates/gearbox-globe/src", "embedded://gearbox_globe/"),
        (
            "crates/gearbox-planet/shaders",
            "embedded://gearbox_planet/../shaders/",
        ),
        ("crates/bevy_weather/src", "embedded://bevy_weather/"),
        ("crates/bevy_biomes/src", "embedded://bevy_biomes/"),
    ] {
        register_sources(
            &mut cache,
            &root.join(directory),
            prefix,
            &mut entries,
            &mut next_id,
        );
    }
    assert!(entries.len() >= 25, "custom shader inventory is incomplete");
    let common: Vec<ShaderDefVal> = [
        "VERTEX_POSITIONS",
        "VERTEX_NORMALS",
        "VERTEX_UVS_A",
        "VERTEX_UVS_B",
        "VERTEX_TANGENTS",
        "VERTEX_COLORS",
        "VERTEX_OUTPUT_INSTANCE_INDEX",
        "MESH_PIPELINE",
        "SHADOW_FILTER_METHOD_HARDWARE_2X2",
    ]
    .into_iter()
    .map(Into::into)
    .chain([
        ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 3),
        ShaderDefVal::UInt("MAX_DIRECTIONAL_LIGHTS".into(), 4),
        ShaderDefVal::UInt("MAX_CASCADES_PER_LIGHT".into(), 4),
        ShaderDefVal::UInt("PER_OBJECT_BUFFER_BATCH_SIZE".into(), 1),
        ShaderDefVal::UInt("AVAILABLE_STORAGE_BUFFER_BINDINGS".into(), 8),
    ])
    .collect();
    let variants: &[&[&str]] = &[
        &[],
        &[
            "AVAILABLE_STORAGE_BUFFER_BINDINGS__GE_3",
            "AVAILABLE_STORAGE_BUFFER_BINDINGS__GE_6",
            "IRRADIANCE_VOLUMES_ARE_USABLE",
            "MULTIPLE_LIGHT_PROBES_IN_ARRAY",
            "MULTIPLE_LIGHTMAPS_IN_ARRAY",
            "ENVIRONMENT_MAP",
        ],
        &[
            "DISTANCE_FOG",
            "STANDARD_MATERIAL_DIFFUSE_TRANSMISSION",
            "STANDARD_MATERIAL_DIFFUSE_OR_SPECULAR_TRANSMISSION",
        ],
        &[
            "VEGETATION_CUTOUT",
            "VEGETATION_SIEVED",
            "GRASS_CHEAP_BACKLIGHT",
        ],
        &["VEGETATION_FLAT"],
        &["DRAW_SPLAT"],
        &["DRAW_SPLAT", "ALPHA_TO_COVERAGE"],
        &["DRAW_FROST", "VIEW_TRANSMISSION_TEXTURE", "DRAW_TERRAIN", "GHOST_EDGE_DATA"],
        &["DRAW_COMPACT_POINTS", "DRAW_SPLAT", "DRAW_FROST", "VIEW_TRANSMISSION_TEXTURE"],
        &["CLOUD_LOD", "CLOUD_OPAQUE", "CLOUD_SINGLE_SAMPLE"],
        &["CLOUD_SINGLE_SAMPLE", "DRAW_FROST", "VIEW_TRANSMISSION_TEXTURE"],
        &["OCTREE_OPAQUE"],
        &["OCTREE_DISCS", "DRAW_FROST"],
        &["OCTREE_TRIANGLES", "DRAW_FROST", "VIEW_TRANSMISSION_TEXTURE"],
        &["PREPASS_PIPELINE"],
        &[
            "PREPASS_PIPELINE",
            "NORMAL_PREPASS_OR_DEFERRED_PREPASS",
            "NORMAL_PREPASS",
            "MOTION_VECTOR_PREPASS",
        ],
        &[
            "PREPASS_PIPELINE",
            "UNCLIPPED_DEPTH_ORTHO_EMULATION",
            "VISIBILITY_RANGE_DITHER",
        ],
    ];
    let mut failures = Vec::new();
    let mut validated = 0;
    for (name, id, supports_prepass) in &entries {
        for (variant, flags) in variants.iter().enumerate() {
            if flags.contains(&"PREPASS_PIPELINE") && !supports_prepass {
                continue;
            }
            let mut defs = common.clone();
            defs.extend(flags.iter().map(|flag| ShaderDefVal::from(*flag)));
            if let Err(error) = cache.get(variant, *id, &defs) {
                failures.push(format!("{name} variant {flags:?}: {error}"));
            }
            validated += 1;
        }
    }
    assert!(
        failures.is_empty(),
        "{} shader failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
    println!(
        "validated {} shaders across {validated} pipeline variants",
        entries.len()
    );
}
