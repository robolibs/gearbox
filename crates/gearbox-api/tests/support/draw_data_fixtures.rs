use serde::Serialize;

pub const FIXTURE_SEED: u32 = 0x5eed_2026;
pub const MAX_FIXTURE_POINTS: usize = 5_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudShape {
    Plane,
    Step,
    CurvedWall,
    NoisyScan,
    Semantic,
}

pub const CLOUD_SHAPES: [CloudShape; 5] = [
    CloudShape::Plane,
    CloudShape::Step,
    CloudShape::CurvedWall,
    CloudShape::NoisyScan,
    CloudShape::Semantic,
];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Sample {
    pub id: u64,
    pub xyz: [f32; 3],
    pub rgba: [u8; 4],
    pub intensity: f32,
    pub class_id: u32,
    pub valid: bool,
}

#[derive(Debug, Serialize)]
pub struct CloudManifest {
    pub schema: &'static str,
    pub shape: CloudShape,
    pub seed: u32,
    pub count: usize,
    pub valid_count: usize,
    pub units: &'static str,
    pub frame: &'static str,
    pub checksum_fnv1a64: String,
    pub queries: Vec<Sample>,
}

fn random(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

pub fn cloud(shape: CloudShape, count: usize, seed: u32) -> Result<Vec<Sample>, String> {
    if count > MAX_FIXTURE_POINTS {
        return Err(format!("fixture point limit is {MAX_FIXTURE_POINTS}"));
    }
    let mut state = if seed == 0 { FIXTURE_SEED } else { seed };
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let x = ((i % 256) as f32 - 128.0) / 16.0;
        let y = ((i / 256) % 256) as f32 / 16.0;
        let sheet = (i / 65_536) as f32 / 8.0;
        let jitter = (random(&mut state) % 1024) as f32 / 32768.0 - 0.015625;
        let xyz = match shape {
            CloudShape::Plane | CloudShape::Semantic => [x, y, 1.0 + sheet],
            CloudShape::Step => [x, y, if x < 0.0 { 1.0 } else { 2.0 } + sheet],
            CloudShape::CurvedWall => [x, x * x / 16.0 + sheet, y],
            CloudShape::NoisyScan => [x, y, 1.0 + sheet + jitter],
        };
        let class_id = (i % 4) as u32;
        let rgba = [
            [230, 80, 40, 255],
            [30, 190, 90, 255],
            [80, 110, 240, 255],
            [210, 180, 50, 255],
        ][class_id as usize];
        let valid = shape != CloudShape::NoisyScan || i % 11 != 0;
        out.push(Sample {
            id: i as u64,
            xyz: if valid { xyz } else { [0.0; 3] },
            rgba,
            intensity: (i % 256) as f32 / 255.0,
            class_id,
            valid,
        });
    }
    Ok(out)
}

pub fn cloud_manifest(shape: CloudShape, seed: u32, samples: &[Sample]) -> CloudManifest {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut add = |bytes: &[u8]| {
        for &byte in bytes {
            hash = (hash ^ byte as u64).wrapping_mul(0x100_0000_01b3);
        }
    };
    for p in samples {
        add(&p.id.to_le_bytes());
        for axis in p.xyz {
            add(&axis.to_le_bytes());
        }
        add(&p.rgba);
        add(&p.intensity.to_le_bytes());
        add(&p.class_id.to_le_bytes());
        add(&[u8::from(p.valid)]);
    }
    let mut ids = vec![0, samples.len() / 2, samples.len().saturating_sub(1)];
    ids.sort_unstable();
    ids.dedup();
    CloudManifest {
        schema: "gearbox.draw.fixture.cloud.v1",
        shape,
        seed,
        count: samples.len(),
        valid_count: samples.iter().filter(|p| p.valid).count(),
        units: "m",
        frame: "world_z_up",
        checksum_fnv1a64: format!("{hash:016x}"),
        queries: ids
            .into_iter()
            .filter_map(|id| samples.get(id).cloned())
            .collect(),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OccupancyLeaf {
    pub tile: [i64; 3],
    pub depth: u8,
    pub morton: u64,
    pub min: [f64; 3],
    pub size_m: f64,
    pub probability: Option<f32>,
}

#[derive(Debug, Serialize)]
pub struct OccupancyManifest {
    pub schema: &'static str,
    pub leaf_resolution_m: f64,
    pub max_depth: u8,
    pub root_span_m: f64,
    pub leaves: Vec<OccupancyLeaf>,
}

pub fn occupancy() -> OccupancyManifest {
    let mut leaves = vec![OccupancyLeaf {
        tile: [-1, 0, 0],
        depth: 0,
        morton: 0,
        min: [-16.0, 0.0, 0.0],
        size_m: 16.0,
        probability: Some(0.9),
    }];
    for child in 0..8u64 {
        let min = [
            (child & 1) as f64 * 8.0,
            ((child >> 1) & 1) as f64 * 8.0,
            ((child >> 2) & 1) as f64 * 8.0,
        ];
        if child == 0 {
            for grandchild in 0..8u64 {
                leaves.push(OccupancyLeaf {
                    tile: [0; 3],
                    depth: 2,
                    morton: grandchild,
                    min: [
                        (grandchild & 1) as f64 * 4.0,
                        ((grandchild >> 1) & 1) as f64 * 4.0,
                        ((grandchild >> 2) & 1) as f64 * 4.0,
                    ],
                    size_m: 4.0,
                    probability: Some(if grandchild % 2 == 0 { 0.1 } else { 0.9 }),
                });
            }
        } else {
            leaves.push(OccupancyLeaf {
                tile: [0; 3],
                depth: 1,
                morton: child,
                min,
                size_m: 8.0,
                probability: match child {
                    1 => Some(0.1),
                    2 => None,
                    3 => Some(0.5),
                    _ => Some(0.9),
                },
            });
        }
    }
    OccupancyManifest {
        schema: "gearbox.draw.fixture.occupancy.v1",
        leaf_resolution_m: 1.0,
        max_depth: 4,
        root_span_m: 16.0,
        leaves,
    }
}
