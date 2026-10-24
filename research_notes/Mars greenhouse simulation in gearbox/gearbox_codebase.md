# Mars greenhouse simulation in gearbox — what the codebase already has

Codebase research on `/home/bresilla/data/code/robolibs/gearbox` (branch `develop`, HEAD `eb13173`, read 2026-10-23). All sources are repository files cited as `path:lines`, relative to the repository root unless absolute. Nothing was launched in a window; the only thing run was the built `target/release/gearbox describe-machines` (read-only) to list the machines. Memory notes in `~/.claude/projects/.../memory/` were used as hints only, and each claim below was checked against the code unless it says otherwise.

## Q1. How is Mars launched today, and what does a Mars world consist of?

### Takeaway
Mars is a procedural planet chosen once per process with `--planet mars` or `GEARBOX_PLANET=mars`. Choosing it swaps a hard-coded `Body` enum, which sets the radius (3389.5 km), gravity (3.711 m/s²), regolith terrain with craters, dunes and streamed boulders, a bare "mars" ground profile with no plants, a `SkyKind::Dust` sky with a fixed sun and no clouds, and turns biomes off. No Mars config or world file exists: the settings are Rust constants plus a few env knobs. The documented launch command is `oslo make run --args='--planet mars --load machines/usd/perseverance_nasa.usdz'`.

### Cited Findings
**Launch path**
- Documented commands: `oslo make run`, `oslo make run --args='--planet mars --load machines/usd/perseverance_nasa.usdz'` and the same with `--planet moon`. The `=` is required because Oslo would treat a separate dash-prefixed value as a flag. — [docs/PLANET_WORLDS.md:9-17](docs/PLANET_WORLDS.md)
- `gearbox run --planet earth|mars|moon` picks the planet, Earth is the default, `GEARBOX_PLANET` does the same for direct launches, and `--world` is a USD scene argument, not the planet selector. The selection is fixed for the life of the process and cannot change at runtime. — [docs/PLANET_WORLDS.md:5-19](docs/PLANET_WORLDS.md)
- The make `run` recipe depends on `build` (`cargo build --release`) and runs `env WINIT_UNIX_BACKEND=$_gb_backend <VULKAN>target/release/gearbox run <args>`. The `sim` recipe runs `gearbox launch <args>` instead, skipping instance management. — [.make.lua:79](.make.lua), [.make.lua:116-137](.make.lua)
- The CLI `run` command:
  - `--planet` is a clap arg (env `GEARBOX_PLANET`, default `earth`, values earth/mars/moon), and `--world USD` / `--load USD` exist. — [crates/gearbox-cli/src/cli/run.rs:20-44](crates/gearbox-cli/src/cli/run.rs)
  - It re-execs the same binary with `launch` and sets `GEARBOX_PLANET`. — [run.rs:90-93](crates/gearbox-cli/src/cli/run.rs)
  - Once the sim answers, it loads `--world` with category WORLD and each `--load` as a MACHINE whose `machine_id` is the file stem. — [run.rs:134-153](crates/gearbox-cli/src/cli/run.rs)
- `gearbox launch [--planet X] [USD ...]` is parsed in `parse_launch_planet`, sets `GEARBOX_PLANET` for the process and logs `planet: <name> radius=… gravity=…`. — [bin/gearbox/src/main.rs:75-76,92,96-119](bin/gearbox/src/main.rs)
- `Body::active()` is a `OnceLock` read from `GEARBOX_PLANET` (Earth when unset). — [crates/gearbox-globe/src/body.rs:26-37](crates/gearbox-globe/src/body.rs)

**Planet constants**
- Mars `radius_m` is 3,389,500, `gravity_mps2` is 3.711 and `home()` is (18.4447° N, 77.4508° E). — [crates/gearbox-globe/src/body.rs:43-57](crates/gearbox-globe/src/body.rs)
- The home datum can be overridden with `GEARBOX_DATUM="lat,lon"`; otherwise it is `Body::active().home()`. — [bin/gearbox/src/globe.rs:108-117](bin/gearbox/src/globe.rs)
- Docs: terrain is "procedural, not a surveyed reconstruction of Jezero"; launch coordinates are reference locations. — [docs/PLANET_WORLDS.md:31](docs/PLANET_WORLDS.md)

**Gravity**
- `PhysicsWorld::with_backend` sets gravity to `(0, -Body::active().gravity_mps2(), 0)`. The default is 120 Hz with 16 solver iterations. — [bin/gearbox/src/physics/world.rs:90-94,117-127](bin/gearbox/src/physics/world.rs)
- The docs say imported USD scenes must not replace Mars gravity, and `oslo make test-planet-gravity` measured 3.711 m/s² after importing gravity scenes. — [docs/PLANET_WORLDS.md:31,45](docs/PLANET_WORLDS.md)
- The machine spec says the opposite: "A `PhysicsScene` prim sets world gravity: the first one loaded wins … Machine assets should not author one". — [specs/MACHINE_SPEC.md:44-46](specs/MACHINE_SPEC.md). PLANET_WORLDS.md and its test say the planet value wins on Mars, so the two docs conflict.

**Terrain and heightmap source**
- The heightmap is fully procedural; no raster is involved. `regolith::height(Body::Mars, p)` adds up:
  - broad fbm (×420 m), hills (×28 m), grit (×0.18 m) and sine dunes (0.32 m) along a fixed wind vector;
  - crater fields on 95 m, 700 m, 9 km, 180 km and 900 km cells;
  - a 3,600 m continental fbm.
  - Source: [crates/gearbox-globe/src/regolith.rs:36-48](crates/gearbox-globe/src/regolith.rs)
- Off Earth, the local terrain is level only very close to home: height = `(regolith − home_raw) * smooth((from_home − 6.0) / 16.0)`. That is flat inside about 6 m and full relief beyond about 22 m. — [crates/gearbox-globe/src/terrain.rs:46-50](crates/gearbox-globe/src/terrain.rs). Earth's flat radius is 30–70 m. — [terrain.rs:7-8,51](crates/gearbox-globe/src/terrain.rs)
- Off Earth, local heights are solved by a radial intersection with the sphere. — [crates/gearbox-globe/src/terrain.rs:61-75](crates/gearbox-globe/src/terrain.rs)
- The drawn ground:
  - is a 1,200 m square (`GEARBOX_TERRAIN_SIZE_M`) with 1 m cells (`GEARBOX_TERRAIN_CELL_M`), 50 m tiles and fine cells within 180 m;
  - follows the view with 220 m slack and a 25 km reach;
  - is backed by physics ground in 64 m heightfield chunks, kept 96 m around every dynamic body;
  - can be set to `GEARBOX_TERRAIN=flat|meadow`.
  - Source: [bin/gearbox/src/terrain.rs:18-68,141-157](bin/gearbox/src/terrain.rs)
- The globe is a CDLOD cube-sphere quadtree with GPU-baked heights, spawned by `spawn_cdlod_planet`. — [bin/gearbox/src/world.rs:330-336](bin/gearbox/src/world.rs), [crates/gearbox-planet/src/planet/quadtree.rs](crates/gearbox-planet/src/planet/quadtree.rs)
- `PlanetConfig::barren` gives Mars an atmosphere with Rayleigh coefficients (18e-6, 10e-6, 5e-6), channel-swapped from Earth's (5.8e-6, 13.6e-6, 33.1e-6), and an 80 km atmosphere shell. — [crates/gearbox-planet/src/config.rs:30-32,54-55,125-127,142-151](crates/gearbox-planet/src/config.rs)
- Boulders:
  - placed on 12 m geodetic cells, 42 % of them occupied, sized 0.14–1.64 m, from 16 templates — [crates/gearbox-globe/src/rocks.rs:6,47-69](crates/gearbox-globe/src/rocks.rs)
  - drawn within 220 m, with triangle-mesh colliders within 80 m, and **skipped within 8 m of home** — [bin/gearbox/src/planet_rocks.rs:18-19,215-217](bin/gearbox/src/planet_rocks.rs)

**Ground material**
- `fields_for(Mars)` returns a layout with default profile `"mars"`, `default_friction` 0.75, no fields and no ways, and zero grass, stubble and concrete-weed densities. — [bin/gearbox/src/environment.rs:37-49](bin/gearbox/src/environment.rs)
- The `"mars"` profile:
  - surface tint (0.24, 0.105, 0.047), wheel-mark recovery 1,800 s, darkening 0.18, `soft_border` 0, `workable: false` — [crates/gearbox-fields/src/bare/mod.rs:526-536](crates/gearbox-fields/src/bare/mod.rs)
  - layers `standing(32.0, 0.0, 0.0, 0.0, blown_clod, pebble)`, where the arguments are stones, clods, tufts and weeds per m², so it is stones only — [bare/mod.rs:194-201](crates/gearbox-fields/src/bare/mod.rs)
  - `mars_ground` builds a `BareGround { planet: Vec4::X, grass: ZERO, … }` — [bare/mod.rs:540-547](crates/gearbox-fields/src/bare/mod.rs)

**Sky, sun and lighting**
- `weather_for(Mars)` sets:
  - `SkyKind::Dust`, cloud coverage 0, cloud-shadow opacity 0, cloud drift 0;
  - wind 3 m/s with gustiness 0.25;
  - a fixed sun override at `(-0.65, 0.65, 0.45)` normalised (about 40° elevation), colour (1.0, 0.94, 0.85);
  - illuminance 58,000 lux, indirect 2,200, haze 18 km, fog colour (0.74, 0.55, 0.40), EV100 13.
  - Source: [bin/gearbox/src/environment.rs:9-35](bin/gearbox/src/environment.rs)
- Daylight for the Dust sky:
  - zenith (0.32, 0.18, 0.105) and horizon (0.72, 0.43, 0.24), both scaled by twilight between −9° and 12°;
  - ambient (0.85, 0.64, 0.47); indirect-bottom (0.20, 0.09, 0.04);
  - no clouds.
  - Source: [crates/bevy_weather/src/daylight.rs:40-59](crates/bevy_weather/src/daylight.rs)
- Dust fog falloff is exponential with density `3.912/(visibility_km·1000)`. — [crates/bevy_weather/src/sky.rs:270-274](crates/bevy_weather/src/sky.rs). The sky scale height is 11.1 km. — [sky.rs:445](crates/bevy_weather/src/sky.rs)
- Docs summary for Mars: "Dust-coloured thin atmosphere, no terrestrial clouds"; vegetation "None"; colours and lighting are "visual approximations, not calibrated radiometry". — [docs/PLANET_WORLDS.md:23-29,47](docs/PLANET_WORLDS.md)

**Plugin wiring and physics regions**
- All plugins are wired in one place (GlobePlugin, PlanetPlugin, WorldPlugin, WeatherPlugin, FieldsPlugin, TerrainPlugin, PlanetRocksPlugin, Sensors, Devices, and so on). `WorldBiomesPlugin` is added **only on Earth**. — [bin/gearbox/src/app.rs:51-110](bin/gearbox/src/app.rs)
- Frames and physics regions:
  - The big_space root grid is the planet frame with 10 km cells; each site is a child grid with one 1e9 m cell. — [bin/gearbox/src/globe.rs:16-19,119-144](bin/gearbox/src/globe.rs)
  - Physics runs in one world. Site *i* occupies the region at x = i·1e6 m (`REGION_SPACING_M`), and a datum reaches 25 km. — [crates/gearbox-globe/src/lib.rs:39-41,163-170](crates/gearbox-globe/src/lib.rs), [globe.rs:154-158](bin/gearbox/src/globe.rs)

**Repository layout (live vs stale)**
- Workspace members: gearbox-api, gearbox-controls, gearbox-fields, gearbox-globe, gearbox-planet, bevy_weather, bevy_biomes, gearbox-cli, bin/gearbox and vendor/bevy_render. — [Cargo.toml:3-14](Cargo.toml)
- `crates/gearbox-{core,physics,viz,editor}` are no longer on disk (`crates/` holds only the eight members above).
- `docs/STATE.md` and `docs/SCHEMA.md` both start with a "Historical design notes, superseded by specs/…" banner. — [docs/STATE.md:3-9](docs/STATE.md), [docs/SCHEMA.md:3-11](docs/SCHEMA.md)
- The memory note listing `gearbox-world`/`gearbox-link` as members is out of date.
- Sibling and external dependencies:
  - patched to local paths: peerbus → `../peerbus`; usd_bevy, openusd and openusd-schemas → `../../OUSD/bevy_openusd/...`; bevy_asset → `../../OUSD/bevy_openusd/vendor/bevy_asset`; bevy_render → `vendor/bevy_render` — [Cargo.toml:72-85](Cargo.toml)
  - git dependencies: big_space, datapod, concord, agentio, mara — [Cargo.toml:21-27,61-71](Cargo.toml)
  - molla physics and sensor crates are git deps pinned at rev f563d54 — [bin/gearbox/Cargo.toml](bin/gearbox/Cargo.toml)

### Inferences
- A Mars greenhouse can start from today's launch unchanged (`gearbox run --planet mars --world greenhouse.usda --load …`). On a pure CLI path, `GEARBOX_PLANET=mars target/release/gearbox run --world <usd>` should be equivalent, per run.rs:20-44.
- The home pad is only about 12 m across before regolith relief returns (terrain.rs:49). A greenhouse larger than that would need one of three things:
  - its own floor collider and foundation in the USD, sitting above the relief;
  - the flat-radius constants made configurable;
  - `GEARBOX_TERRAIN=flat`. Its interaction with the Mars globe was not checked.
- The boulder exclusion around home is also only 8 m (planet_rocks.rs:215), so rocks would appear inside a large greenhouse footprint unless the exclusion is widened or made per-structure.
- On Mars the sun is a fixed override (environment.rs:24-27) and nothing advances it, so a sol-length day/night cycle does not exist (see Q5).

### Gaps
- I did not run Mars here, so visuals, frame rate and the actual sun elevation were not observed.
- There is no Mars dust-storm, atmospheric-opacity (tau) or seasonal model. Mars sky, illuminance and haze are single constants, not functions of time or location.
- I could not determine from code alone whether `GEARBOX_TERRAIN=flat` interacts cleanly with the Mars globe and the barren ground mask.

## Q2. How are worlds, sites and scenes described, how would a greenhouse interior be added, and can static structures be placed as USD?

### Takeaway
There is no world-file schema. A scene is assembled from three things:
- the procedural planet;
- an optional field-layout JSON, which only applies at the home site and is axis-aligned rectangles plus polyline "ways";
- any number of USD loads over `/gearbox/usd/load`, with category static, machine, variant, world or terrain. A CLI TOML manifest can batch these loads.

Static structures are supported as world-category USDs: every loaded stage gets its USD physics colliders, UsdLux lights map to Bevy lights, and UsdPreviewSurface opacity, ior and transmission map to blended or transmissive materials. A greenhouse is therefore best authored as a world USD. "static" props are the wrong category: they get a dynamic capsule body that settles and then freezes.

### Cited Findings
**Load categories and the request**
- Categories: static 0, machine 1, variant 2, world 3, terrain 4. — [crates/gearbox-api/src/wire/host.rs:5-32](crates/gearbox-api/src/wire/host.rs)
- The `UsdLoad` request carries x, y, z, yaw_deg, category, flags and props (`id`, `path`, `machine_id`, `variant.<n>`). — [crates/gearbox-api/src/wire/host.rs:265-276](crates/gearbox-api/src/wire/host.rs), [docs/SCHEMA.md:23-44](docs/SCHEMA.md) (historical, but the categories still match)
- The loader names world/terrain roots `UsdWorld[id]` and every other root `UsdLoad[id]`. Re-sending the same id and path moves the object in place. — [crates/gearbox-api/src/plugin/loader.rs:166-178,202-206](crates/gearbox-api/src/plugin/loader.rs)
- Roots loaded through the API are always parented to **site 0 (home)**; roots loaded from the CLI go to the site in view, or to their `InDatum`. — [bin/gearbox/src/globe.rs:223-236](bin/gearbox/src/globe.rs)

**Static props**
- Roots named `UsdLoad[` are snapped to the terrain height plus their requested Y and then aligned by their bounding box. — [bin/gearbox/src/world.rs:733-757](bin/gearbox/src/world.rs)
- Each gets a **dynamic capsule body** (density 80, linear damping 4, angular damping 8) that freezes to Fixed once settled. — [world.rs:906-958,1073-1100](bin/gearbox/src/world.rs)
- Its settled pose is then published once as a `pose` SceneEvent. — [world.rs:960-1000](bin/gearbox/src/world.rs), [docs/SCHEMA.md:88-113](docs/SCHEMA.md)

**Physics and units for any loaded stage**
- Every projected stage goes through `attach_projected_stages`, which reads stage metadata, attaches colliders, bodies and joints, and compiles a `LoadedModel`. — [bin/gearbox/src/physics/mod.rs:145-197](bin/gearbox/src/physics/mod.rs)
- A collider is any prim with `PhysicsCollisionAPI` and belongs to the nearest body above it. Shapes are Cube, Sphere, Capsule, Cylinder, Plane and Mesh. A *static* Mesh with approximation `none` becomes a triangle mesh. — [specs/MACHINE_SPEC.md:250-270](specs/MACHINE_SPEC.md)
- Stage units: `metersPerUnit` **defaults to 0.01 when unauthored**, so it must be set to 1. `upAxis` Z is rotated into the Y-up world. — [specs/MACHINE_SPEC.md:34-41](specs/MACHINE_SPEC.md)

**Terrain-category USDs**
- A USD terrain root keeps the flat fallback until its collider is ready. — [bin/gearbox/src/world.rs:471-486](bin/gearbox/src/world.rs)
- After that the procedural terrain retires (`retire_for_usd_terrain`). — [bin/gearbox/src/terrain.rs:933](bin/gearbox/src/terrain.rs)

**Lights and glass in usd_bevy**
- usd_bevy maps lights as follows:
  - `DistantLight` → DirectionalLight;
  - `SphereLight`/`DiskLight` → PointLight, or SpotLight when a cone angle is authored;
  - `RectLight`/`CylinderLight` → a PointLight *approximation* plus a `UsdAreaLight` marker.
  - Intensity is mapped only approximately. — [/home/bresilla/data/code/OUSD/bevy_openusd/crates/usd_bevy/src/route/light.rs:1-13](/home/bresilla/data/code/OUSD/bevy_openusd/crates/usd_bevy/src/route/light.rs)
- gearbox uses CPU light clustering, which "still serves any point or spot lights a USD brings". — [bin/gearbox/src/app.rs:120-128](bin/gearbox/src/app.rs)
- usd_bevy materials:
  - `opacity` < 1 or an opacity texture → `AlphaMode::Blend`; `opacityThreshold` → `Mask`;
  - `ior` is copied;
  - `transmission` → `specular_transmission`, with the transmission colour used as a tint. — [/home/bresilla/data/code/OUSD/bevy_openusd/crates/usd_bevy/src/route/material.rs:420-465](/home/bresilla/data/code/OUSD/bevy_openusd/crates/usd_bevy/src/route/material.rs)

**Existing world assets**
- `bin/gearbox/assets/world/` holds `yard.usda` (machines plus attachments), `sensor_suite.usda`, `sensor_yard.usda`, `device_yard.usda`, `flatland.usd`, `peafield.usd`, `terrain.usd`, `slope10_gravity.usda`, `tractor_cameras.usda` and `spray_drone.usda` (directory listing).
- `bin/gearbox/assets/demarke/barn.usdc` (59.8 MB) is a building, built by `scripts/build_demarke_barn.py`. — memory note project_barn_example (hint, not re-verified)
- `yard.usda` shows a world composed by USD references to machine assets, plus a `GearboxAttachmentAPI` scope. — [bin/gearbox/assets/world/yard.usda:1-44](bin/gearbox/assets/world/yard.usda)

**Spawn manifests and CLI**
- Manifest format:
  - `[[spawn]]` entries take `kind` (usd, machine, world, terrain or marker), `path`, `id`/`machine` (alias `ns`), `at = [x, y, z]`, `yaw`, `variants` and `tyres`;
  - `[[attach]]` entries take master, slave, hitch, coupler and teleport.
  - Source: [crates/gearbox-cli/src/cli/spawn.rs:278-298,450-470](crates/gearbox-cli/src/cli/spawn.rs)
- CLI commands: `spawn usd|machine|world|terrain|marker|from|move`. `scene reset` means "Clear, then reload the world and every recorded manifest". — [docs/CLI.md:34-46,77](docs/CLI.md)

**Field layout JSON**
- Shape:
  - top level `{default, default_friction?, fields[], ways[]}`;
  - each `FieldSpec` has `name, profile, min[2], max[2], wear?, way[], way_width?, tyre?, friction?, yaw`;
  - each `WaySpec` has `name, points, width, wear, tyre?`.
  - Source: [crates/gearbox-fields/src/layout.rs:957-987,1028-1041,1187-1197](crates/gearbox-fields/src/layout.rs)
- It is read from `GEARBOX_FIELD_LAYOUT` (a file path) — [layout.rs:1211-1220](crates/gearbox-fields/src/layout.rs) — or at runtime via `/gearbox/fields/layout` — [crates/gearbox-api/src/topics.rs:16-17](crates/gearbox-api/src/topics.rs)
- It is only laid at home; other sites get the default profile without fields or ways. — [bin/gearbox/src/globe.rs:381-418](bin/gearbox/src/globe.rs)
- Bundled examples live in `crates/gearbox-fields/src/layouts/*.json`, e.g. mixed.json.

**Sites**
- A site is an internal tangent frame. `travel` founds a site or joins one within 15 km, `join_watched_site` follows the camera, and `reanchor_machines` handles machines that drive away. Scripted views use `GEARBOX_CAMERA_LLA`. — [bin/gearbox/src/globe.rs:21-47,289-352,598-710](bin/gearbox/src/globe.rs)
- "Sites" are internal; users address places by lat/lon or by north/up/east metres from home. — memory note project_globe_sites (hint)

### Inferences
- **Recommended route for a greenhouse interior.** Author a single world USD and load it with `gearbox spawn world greenhouse.usda`, or with `--world` at launch. The stage should set `metersPerUnit=1` and `upAxis="Z"`, and contain:
  - a structural frame and floor with `PhysicsCollisionAPI` and no rigid body, so they are static;
  - glazing with UsdPreviewSurface `opacity`/`ior`/`transmission`;
  - raised beds as static colliders;
  - grow lights as UsdLux Sphere or Rect lights.
- A world load is not terrain-snapped (the snap code only matches `UsdLoad[` names, world.rs:738), so the greenhouse's Y must be authored to sit on the Mars ground at home. Because the home pad is only about 6 m flat, a foundation slab would be needed.
- API world loads always land in the home site (globe.rs:233-235). A greenhouse away from home would need either a CLI load or a datum/`--lla` feature for world loads.
- "Adding a new site" in the user's sense means placing a world USD at a lat/lon. Internal sites are created automatically and are not authored.

### Gaps
- Not verified:
  - whether blended or transmissive glass casts sun shadows in Bevy 0.19 here, and whether Bevy transmission works inside the mara viewport;
  - interior exposure: EV100 is fixed at 13 on Mars (environment.rs:33), and the effect of fixed exposure on interior lights was not checked;
  - how big a triangle-mesh static collider can get before molla's sensor snapshot or contact cost grows.
- There is no "greenhouse" or "building" category or schema. Glass would also be a collider, which matters for ray-cast sensors (see Q6).

## Q3. Vegetation: how are plants placed, rendered and instanced? Is there any notion of growth, plant state or per-plant data? How many plants can it handle?

### Takeaway
Every plant (grass blades, stones, Poly Haven clumps) is a **stateless GPU-procedural instance**. Its position, yaw, scale and whether it survives are re-derived every frame from a hash of the instance index, the 16 m chunk corner and the pack salt, inside the vertex or compute shader.

Persistent "state" exists only per texel, never per plant:
- a wheel/trample map, which recovers;
- a tool work map, which is permanent and only written by a disc harrow.

There is no growth, no plant identity, no CPU-side plant list, no plant colliders and no shadow casting. On Mars the whole field cover is configured to zero grass.

### Cited Findings
**Plugin and densities**
- `FieldsPlugin` registers ClumpsPlugin, GrasslandPlugin, BarePlugin, HarvestedWheatPlugin, ConcretePlugin, TillagePlugin, VegetationPlugin, BladePlugin and SievePlugin. — [crates/gearbox-fields/src/lib.rs:82-131](crates/gearbox-fields/src/lib.rs)
- Default densities are 6,000/m² (`GEARBOX_GRASS_DENSITY`, `GEARBOX_STUBBLE_DENSITY`) and 36 concrete weeds/m² (`GEARBOX_CONCRETE_WEEDS`). — [lib.rs:50-80](crates/gearbox-fields/src/lib.rs)

**Layer model**
- `VegetationLayer` fields are: shader, `template: fn() -> Mesh`, density, fade_start/end, inverse_square_thinning, albedo, lod_band, follow_grass, cutout, way_only, worked_only, `blade: Option<BladeLayer>` and `sieve`. A layer has **no growth, age or per-instance buffer field**. — [crates/gearbox-fields/src/profile.rs:49-80](crates/gearbox-fields/src/profile.rs)
- `BladeKind` and `BladeLayer` describe GPU-sown blades. — [profile.rs:82-104](crates/gearbox-fields/src/profile.rs)
- `FieldProfile` holds name, wheel_response, layers, a ground factory, tread, surface_tint, soft_border and workable. — [profile.rs:183-203](crates/gearbox-fields/src/profile.rs)
- `GEARBOX_VEGETATION_RANGE` scales the fades. — [profile.rs:225-233](crates/gearbox-fields/src/profile.rs)

**Registered profiles**
- `grassland` — [grassland/mod.rs:111](crates/gearbox-fields/src/grassland/mod.rs)
- `harvested_wheat` — [harvested_wheat/mod.rs:248](crates/gearbox-fields/src/harvested_wheat/mod.rs)
- `concrete` — [concrete/mod.rs:79](crates/gearbox-fields/src/concrete/mod.rs)
- `ploughed`, `dirt`, `green-lane`, `track`, `road`, `sand`, `mars`, `moon` — [bare/mod.rs:480-536](crates/gearbox-fields/src/bare/mod.rs)
- There is no standing-crop profile: no lettuce, tomato or wheat-in-ear. `harvested_wheat` is stubble.

**Poly Haven clumps**
- Seven packed models, each made by a `pack!` macro constructor `fn(density, fade_end) -> VegetationLayer`: `bermuda`, `meadow_tufts` (grass_medium_01), `sorrel`, `celandine`, `flat_weeds` (weed_plant_02), `dandelion`, `nettle`. Each has `cutout: true` and an embedded albedo. — [crates/gearbox-fields/src/clumps/mod.rs:10-64](crates/gearbox-fields/src/clumps/mod.rs)
- Meshes use a VEG1 binary layout (position, normal, uv, colour, u32 indices), with at most 15 variants. — [clumps/mod.rs:66-107](crates/gearbox-fields/src/clumps/mod.rs)
- The raw CC0 sources are 8 models in `bin/gearbox/assets/vegetation/`. Their README says `scripts/pack_vegetation.py` builds the packs. — [bin/gearbox/assets/vegetation/README.md](bin/gearbox/assets/vegetation/README.md)
- **That script is not in the working tree** (`scripts/` holds only `build_demarke_barn.py` and `robot_contract/`). `git log` shows it last touched in commit 9d7acbc.

**Placement maths (clumps)**
- Per instance:
  - `id = pcg(index ^ chunk_seed ^ salt)`;
  - `base = corner + (rand(id,1), rand(id,2)) * chunk_size`;
  - kept if `rand(id,9) < mix(loose, 1, patchiness)*(1-bared) + verge*0.3`;
  - `scale = (0.8 + 0.4*rand(id,5)) * coverage`, where coverage is a distance fade;
  - yaw is `rand(id,4)*2π`;
  - wind lean comes from `globals.time`.
- No per-instance stored attribute is read. — [crates/gearbox-fields/src/clumps/shaders/clumps.wgsl:143-210](crates/gearbox-fields/src/clumps/shaders/clumps.wgsl)
- Chunks are 16 m (`CHUNK_M`). Wheel/track maps use 8 texels/m with at most 8,192 texels per side. — [crates/gearbox-fields/src/runtime.rs:293-297](crates/gearbox-fields/src/runtime.rs)

**Blades and sieve**
- Grass blades are sown on the GPU by a compute cull into records (`GRASS_BLADES`: stride 16 words, **capacity 1,500,000 records per view**) and drawn indirectly. — [crates/gearbox-fields/src/grassland/mod.rs:308-314](crates/gearbox-fields/src/grassland/mod.rs); shaders [grassland/shaders/blade_cull.wgsl](crates/gearbox-fields/src/grassland/shaders/blade_cull.wgsl), [blade_draw.wgsl](crates/gearbox-fields/src/grassland/shaders/blade_draw.wgsl); [crates/gearbox-fields/src/blades.rs](crates/gearbox-fields/src/blades.rs)
- A per-chunk compute sieve passes survivors to the draw. — [crates/gearbox-fields/src/sieve.rs](crates/gearbox-fields/src/sieve.rs)
- The records are rebuilt each frame per view. They are a culling cache, not persistent plant state. This is inferred from the "sow per view" design; the memory note project_grass_perf says the vegetation pass went from 71 ms to about 10 ms (hint, not re-measured).

**follow_grass and shared WGSL**
- `follow_grass` > 0 makes a layer follow the ground's 9 m grass patches, keeping that share on bare ground. — [profile.rs:62-65,106-112](crates/gearbox-fields/src/profile.rs), [clumps.wgsl:160-168](crates/gearbox-fields/src/clumps/shaders/clumps.wgsl)
- `bare/shaders/cover.wgsl` (691 lines) holds the shared `pcg/rand/lattice/taken/worn/washed_into/way_beyond…`. — [crates/gearbox-fields/src/bare/shaders/cover.wgsl](crates/gearbox-fields/src/bare/shaders/cover.wgsl)
- `src/shader_contract.rs` is a test that enforces those functions are imported, never re-declared. — [crates/gearbox-fields/src/shader_contract.rs](crates/gearbox-fields/src/shader_contract.rs)

**Persistent per-texel state**
- `WheelMapParams` has `recovery_seconds`, bend and darkening, so the trample map decays. — [crates/gearbox-fields/src/profile.rs:23-47](crates/gearbox-fields/src/profile.rs)
- The tillage work map: "A worked texel … keeps it for good". The only `ToolKind` is `DiscHarrow`, chosen by `gearbox:machine:kind` ∈ {disc_harrow, disc, harrow, short_disc}. — [crates/gearbox-fields/src/tillage/mod.rs:1-44](crates/gearbox-fields/src/tillage/mod.rs)
- Layers with `worked_only` appear only where a tool has worked the ground. — [tillage/mod.rs:219](crates/gearbox-fields/src/tillage/mod.rs), [profile.rs:71-73](crates/gearbox-fields/src/profile.rs)

**Terrain coupling**
- Vegetation stands on the host's `CoverTerrain` HeightGrid heightmap. A `HeightSource` serves plants outside that grid. — [crates/gearbox-fields/src/host.rs:8-26](crates/gearbox-fields/src/host.rs)
- `CoverTerrainRoots` gives USD terrain meshes the bare soil material only. — [host.rs:59-62](crates/gearbox-fields/src/host.rs)
- One global `CoverWind` leans every plant, synced from the weather. — [host.rs:36-57](crates/gearbox-fields/src/host.rs), [bin/gearbox/src/environment.rs:112-123](bin/gearbox/src/environment.rs)

**Render phase and cameras**
- Vegetation is drawn only in the `Transparent3d` phase (`add_render_command::<Transparent3d, DrawVegetation>`). No shadow-phase command exists. — [crates/gearbox-fields/src/render.rs:198,219-281](crates/gearbox-fields/src/render.rs); memory note project_plant_assets, "Nothing in the vegetation pipeline casts a shadow" (consistent with the code)
- `NoVegetation` keeps vegetation out of a camera. Lean ("optimized") sensor cameras get it. — [crates/gearbox-fields/src/render.rs:88-90](crates/gearbox-fields/src/render.rs), [bin/gearbox/src/sensor_cameras.rs:153-162](bin/gearbox/src/sensor_cameras.rs)

**Biomes**
- `bevy_biomes::Plants` is a *recipe only*: `name, per_square_metre, reach_m, height_m`. "The blades themselves are drawn elsewhere". — [crates/bevy_biomes/src/layer.rs:12-21](crates/bevy_biomes/src/layer.rs)

### Inferences
- **Scale.** The renderer can cover 1e5–1e6 visible blades (6,000 blades/m² default, 1.5 M records per view). Clumps are "hundreds in view" per the memory hint. A greenhouse of a few thousand individually tracked crop plants is small for the renderer. The problem is architectural: no instance has an identity or a stored attribute, so plants cannot be addressed, grown, harvested or reported.
- **What per-plant plants would need.** Either a storage-buffer layer that reads per-plant records (position, species, growth stage, health) uploaded from a CPU-side ECS list, replacing the hash-derived base/scale; or plants as ordinary entities (USD or Bevy meshes, possibly GPU-instanced) driven by a growth system. The existing `follow_grass`, `worked_only` and `way_only` mechanics show how a layer reads a per-texel map. A "planted/growth-stage" texture map, analogous to the work map, is a cheap intermediate: per texel, not per plant.
- **Raised beds and tables.** Plants follow only the procedural ground heightmap (host.rs:8-26). Plants in raised beds or on benches inside a USD greenhouse would not stand on the bed tops unless the bed geometry is folded into the cover heightmap, or a new placement path is written.
- **Wind indoors.** On Mars `CoverWind` is 3 m/s with gusts (environment.rs:22-23). Indoor plants would sway in Mars wind unless wind becomes regional, or is zeroed when the scene is indoor-only.

### Gaps
- I did not confirm the exact per-frame cost or limits of clumps, or the maximum number of vegetation layers per profile.
- I did not check whether `HeightSource` could be pointed at USD bed meshes.
- `GRASS_PLAN.md` (untracked, repo root) and the memory performance numbers were not re-validated.

## Q4. Biomes and weather: what do bevy_biomes and bevy_weather provide, and are there Mars-specific settings?

### Takeaway
`bevy_weather` owns the sun, the daylight colour model, haze/fog, volumetric clouds with cloud shadows, wind and the `SkyLight` direct/diffuse split. Its Mars support is the `SkyKind::Dust` branch: a hand-tuned rust sky palette, exponential haze, an 11.1 km scale height and no clouds. The Mars values themselves live in gearbox's `environment.rs`, not in the crate.

`bevy_biomes` provides biome names, soft or hard border blending, climate "damp" noise and drifting motes (dust, pollen and others). It has **no Mars biome** and is **not added on Mars at all**.

### Cited Findings
**bevy_weather**
- `WeatherSettings` fields: sky_kind, calendar, sun_override, sun_position, sun_color, illuminance, indirect_intensity, bloom, fog_color, haze_visibility_km, clouds (CloudsConfig), cloud_drift_mps, planet_from_site, wind_heading/speed/gustiness. — [crates/bevy_weather/src/lib.rs:26-50](crates/bevy_weather/src/lib.rs)
- Env knobs: `GEARBOX_SUN_ELEVATION`, `GEARBOX_SUN_AZIMUTH`, `GEARBOX_HAZE_KM`, `GEARBOX_CLOUD_COVER`, `GEARBOX_CLOUD_SHADOWS`. — [lib.rs:52-124](crates/bevy_weather/src/lib.rs)
- The sky kinds are `SkyKind::{Earth, Dust, Airless}`. — [lib.rs:147-153](crates/bevy_weather/src/lib.rs)
- `SkyLight::from_cover(cover)` gives `mean_direct`, `diffuse_fraction` and `diffuse_gain` (Kasten–Czeplak style). — [lib.rs:171-205](crates/bevy_weather/src/lib.rs)
- The Dust daylight palette (Q1) — [crates/bevy_weather/src/daylight.rs:40-59](crates/bevy_weather/src/daylight.rs)
- Dust fog — [sky.rs:270-274](crates/bevy_weather/src/sky.rs); scale height — [sky.rs:445](crates/bevy_weather/src/sky.rs)
- `LEAN_LAYER` (3): sensor cameras are lit by a shadowless twin of the sun. — [lib.rs:17-20](crates/bevy_weather/src/lib.rs)
- Mars settings applied by gearbox (dust sky, clouds 0, wind 3 m/s, fixed sun, 58 klx, haze 18 km, rust fog) — [bin/gearbox/src/environment.rs:13-35](bin/gearbox/src/environment.rs). Tested by `planet_skies_and_surfaces_are_distinct_and_barren`. — [environment.rs:51-75](bin/gearbox/src/environment.rs)
- Clouds come from evroon's volumetric clouds (MIT), absorbed as `bevy_weather::clouds` — [crates/bevy_weather/src/clouds/UPSTREAM.md](crates/bevy_weather/src/clouds/UPSTREAM.md). They are off on Mars.

**bevy_biomes**
- Exports Biome, Border, Field, Land, Climate, damp, Biomes, Mix, Region, Cover, Covers, MoteKind, Motes, Plants, AmbientWind and MoteBudget. — [crates/bevy_biomes/src/lib.rs:1-32](crates/bevy_biomes/src/lib.rs)
- `Land` includes `Desert`, `ColdDesert`, `Volcanic`, `IceCap` and `Barren`. `Field` includes Wheat, Maize, Potato, Orchard, Vineyard and Seeded. These are names only. — [crates/bevy_biomes/src/biome.rs:13-55](crates/bevy_biomes/src/biome.rs)
- Built-in covers include Grassland (petal and leaf motes), Desert (Dust motes, 16,000/ha), Steppe, Stubble and others. — [crates/bevy_biomes/src/layer.rs:100-180](crates/bevy_biomes/src/layer.rs)
- The memory note says Covers held only Grassland, Stubble and Yard; the code now has more.
- `WorldBiomesPlugin` maps profiles to biomes (`harvested_wheat`→Stubble, `concrete`→Yard, otherwise Grassland) and carries the wind. — [bin/gearbox/src/biomes.rs:1-30](bin/gearbox/src/biomes.rs). It is registered only when `Body::active() == Earth`. — [bin/gearbox/src/app.rs:108-110](bin/gearbox/src/app.rs)
- Climate: `Climate { sand, lush, parched_land, dry_land, damp_land }`, driven by `damp(place)` noise. — [crates/bevy_biomes/src/climate.rs:55-63](crates/bevy_biomes/src/climate.rs)

### Inferences
- For Mars dust in the air, the existing `MoteKind::Dust` cover (Desert) could be reused if `WorldBiomesPlugin` were enabled on Mars with a Mars cover. Today it is Earth-gated.
- Inside a greenhouse the weather's outdoor values (wind, haze, sun) apply everywhere: there is no indoor or outdoor volume concept. A greenhouse microclimate (air temperature, humidity, CO₂, PAR) has **no representation** in either crate.

### Gaps
- Neither crate models temperature, humidity, CO₂, pressure, radiation dose or dust deposition on glazing. I found no field or resource for any of them.

## Q5. Time: how does the sim clock work, and could plant growth run on accelerated time?

### Takeaway
Physics runs fixed steps (default 120 Hz, at most 12 steps per frame) paid for out of real frame time. Play, pause and step exist, but **there is no time scale**: the CLI `scene speed` explicitly errors with "the sim has no time scale yet". Simulated seconds accumulate in `PhysicsWorld.simulated_seconds`; shaders use Bevy's real `globals.time`.

The sun clock (hour, day, latitude) is an Earth NOAA formula that only changes when a UI slider moves it, and on Mars the sun is a fixed override. Plant growth over days or sols would therefore need its own scaled growth clock. The machinery to accelerate physics does not exist, except for a scripted `scene step N` budget, which runs up to 12 ticks per frame regardless of wall time.

### Cited Findings
**Physics stepping**
- `DEFAULT_STEP_HZ = 120.0`, overridable with `GEARBOX_PHYSICS_HZ` (≥ 30). `MAX_STEPS_PER_FRAME = 12`: "below that frame rate physics slows down instead of spiralling". — [bin/gearbox/src/physics/world.rs:90-94,117-122](bin/gearbox/src/physics/world.rs)
- `PhysicsWorld` holds step_hz, accumulator, pending_steps, `simulated_seconds` ("never rewinds"), pause_after_steps and ticks. — [world.rs:61-73](bin/gearbox/src/physics/world.rs)
- `plan_physics_steps`:
  - accumulator += `time.delta_secs_f64().min(0.25)`;
  - steps = accumulator/dt, capped at 12;
  - when a `PhysicsStepBudget` is set, it runs `min(left, 12)` steps per frame regardless of real time, then pauses.
  - Source: [world.rs:463-491](bin/gearbox/src/physics/world.rs)
- `step_physics` runs the `ControlTick` schedule before each step. — [world.rs:495-510](bin/gearbox/src/physics/world.rs)

**Clock API**
- Clock ops: GET, PAUSE, PLAY, TOGGLE, SHUTDOWN, STEP. `ClockState` is `{step, paused}`. — [crates/gearbox-api/src/wire/host.rs:34-65,171-183](crates/gearbox-api/src/wire/host.rs)
- Topics: `/gearbox/scene/clock` and `/gearbox/scene/clock/state`. — [crates/gearbox-api/src/topics.rs:4-5](crates/gearbox-api/src/topics.rs)
- `scene speed <FACTOR>` → `CliError::unsupported("the sim has no time scale yet …; use scene step or scene pause")`. — [crates/gearbox-cli/src/cli/scene.rs:76-78](crates/gearbox-cli/src/cli/scene.rs), [docs/CLI.md:73](docs/CLI.md)
- A fresh instance starts paused at step 0, so `gearbox scene play` is needed. — memory note project_launch_from_shell (hint)

**Clocks that use simulated time**
- Sensors sample at "their own simulated-time rates" — [bin/gearbox/src/sensors.rs:1-3,400](bin/gearbox/src/sensors.rs)
- Joint state stamps `sim_time_s: physics.simulated_seconds` — [bin/gearbox/src/joint_control/api.rs:299](bin/gearbox/src/joint_control/api.rs)
- Devices use the same — [bin/gearbox/src/devices.rs:708,781](bin/gearbox/src/devices.rs)
- Controllers have `updateRateHz` on a simulation-time phase accumulator — [specs/CONTROLLER_SPEC.md:86-96](specs/CONTROLLER_SPEC.md)

**Sun and calendar**
- `SolarCalendar { hour: 15, day: 172, latitude: 52 }` with an approximate NOAA solar direction over a 365-day Earth year and Earth declination. — [crates/bevy_weather/src/calendar.rs:3-40](crates/bevy_weather/src/calendar.rs)
- The only writers of `calendar.hour/day/latitude` are the Environment pane sliders followed by `apply_calendar()`. Nothing advances the hour automatically. — [bin/gearbox/src/host/panes/environment.rs:17-80,214-228](bin/gearbox/src/host/panes/environment.rs); grep of `calendar.hour` across bin/ and crates/
- On Mars `sun_override = true` with a fixed direction. — [bin/gearbox/src/environment.rs:24-27](bin/gearbox/src/environment.rs)
- The wheel-mark clock uses `Time::elapsed_secs_wrapped` (1 h wrap) — memory note project_terrain (hint). Shaders animate wind with `globals.time` — [clumps.wgsl:208](crates/gearbox-fields/src/clumps/shaders/clumps.wgsl)

### Inferences
- Plant growth runs on days to weeks, while robot physics runs at 120 Hz. The workable design is a **separate growth clock**: sim-days = f(simulated_seconds) × a growth-acceleration factor. It would advance plant state in an ordinary Update system (or one keyed on `ControlTick` ticks) without speeding up physics.
- Speeding up physics itself is capped by `MAX_STEPS_PER_FRAME`, at about 12 × fps / 120 times real time (about 6× at 60 fps), and only through the step-budget path.
- A Mars sol (≈ 24.66 h) day/night cycle and seasons (≈ 668.6 sols per year) are not modelled. The calendar formula is Earth's. These Mars figures are general knowledge and are not in the repo.

### Gaps
- No sim-time-to-calendar coupling exists, and there is no API to set the time of day. Sun position can only be set by env vars at startup or by UI sliders. Moving the sun over the API is not possible.

## Q6. Robots: which machines exist, which sensors, how are they driven, and could an arm or gantry be modelled?

### Takeaway
There are 24 USD packages in `machines/usd`, 7 built-in assets in `bin/gearbox/assets`, and a `spray_drone` world asset. They cover:
- tractors, trailers and implements;
- small UGVs: Husky, Hunter, Tracer;
- weeding robots: Ekobot, Weedunit;
- barn robots: Lely Juno, Discovery, Triomatic, Feedr;
- a copter;
- the **Perseverance rover** (`kind=planetary_rover`), which has a 5-joint arm with drill, mast cameras and 11 sensor frames.

Sensors include IMU, LiDAR, camera (colour from a Bevy render; depth and recognition from a Molla trace of *physics colliders*), GPS, compass, distance, light, radar, touch and radio. Robots are driven over agentio/peerbus by `cmd_vel` twists or per-joint `joint_command` under leases.

Arms and gantries are modellable: revolute, prismatic and D6 joints, world-anchored joints and `builtin:joint_position` controllers are conformance-tested against Isaac, including an `e02_arm` fixture.

### Cited Findings
**Machines (from `target/release/gearbox describe-machines <file>`)**

| Package | kind | Controllers | Sensor frames |
|---|---|---|---|
| agilex_hunter | ackermann_ugv | ackermann_cmd_vel | — |
| clearpath_husky | skid_steer_ugv | diff_drive_cmd_vel | imu_link |
| agreenculture_ceol | tracked_tool_carrier | tracked_cmd_vel, hitch, 2× joint_position | lidar, gnss, imu |
| agrointelli_robotti | autonomous_tool_carrier | ackermann, hitch, pto, 5× joint_position | camera_front/rear, lidar, imu, gps |
| agxeed_agbot | tracked_tractor | — | 2 cameras, 2 gnss, lidar, imu |
| ekobot_robot | autonomous_weeding_robot | ackermann, hitch, 12× joint_position | — (12 `tool` frames) |
| weedunit | implement | 9× joint_position | — (9 tools) |
| weedunit_trailer | weeding_trailer | — | — |
| weedunit_trailer_test | weeding_trailer | — | `weed_camera` |
| kubota_fuel_cell | tractor | — | 4 cameras, lidar_front |
| lely_exos | grass_harvester | — | 2 cameras, gnss, imu |
| lely_juno_max | feed_pusher | — | camera, imu |
| lely_discovery_collector | manure_robot | — | ultrasonic, imu |
| trioliet_triomatic | feed_robot | — | antenna, imu |
| sieplo_feedr | feed_robot | — | imu |
| oxbo_harvester | harvester | 9× joint_position, 4× joint_velocity | — |
| ropa_harvester | trailed_harvester | 188 bodies, section_control | — |
| perseverance_nasa | planetary_rover | 10× joint_position (arm turret, shoulder yaw/pitch, elbow, wrist, mast, HGA…), joint_velocity (drill_spin), ackermann_cmd_vel; 29 bodies, 30 joints | navcam L/R, mastcam L/R, 4 front hazcams, 2 rear hazcams, imu_link |
| bin/gearbox/assets sprayer.usd | sprayer | joint_position, joint_velocity, rate_control, section_control | — |
| bin/gearbox/assets world/spray_drone.usda | drone | — | imu, gps, attitude |

  The other packages (fendt/kubota tractors, trailers, spader, disc harrow, and the bin/gearbox/assets husky/hunter/tracer/tractor/trailer/oxbo) are in the full summary. The raw JSON is in the session scratchpad, `desc/*.json`. — command output; sources are [machines/usd/](machines/usd) and [bin/gearbox/assets/](bin/gearbox/assets)
- Blender sources are `machines/blender/*.blend`, including `perseverance_nasa.blend` and `weedunit.blend` (directory listing).

**Sensor kinds and specs**
- Kinds: imu, lidar, camera (alias range_finder), accelerometer, gyro, inertial_unit, compass, gps, distance (laser/infra_red/sonar), light, position, radar, touch (bumper/force/force3d), receiver/emitter (radio/infrared/serial). — [bin/gearbox/src/links.rs:153-193,236-256](bin/gearbox/src/links.rs)
- `SensorSpec` covers rate_hz, lidar columns/rows/fov/range, camera width/height, render mode, color/depth/recognition flags, rays/aperture and radio channel. Max camera side is 4,096; max lidar pulses 2²⁰. — [links.rs:258-327](bin/gearbox/src/links.rs)
- `CameraRender`:
  - `Geometry` is a Molla ray trace of collision shapes with flat albedo;
  - `Optimized` is a Bevy render "without grass, weather, shadows";
  - `Full` is the same pipeline as the viewer.
  - **"Depth always comes from the Molla trace of the collision scene."** — [links.rs:287-313](bin/gearbox/src/links.rs)
- Sensors snapshot the Molla physics scene geometry "once per scene revision". IMU and LiDAR come from Molla GPU sensors. — [bin/gearbox/src/sensors.rs:1-8](bin/gearbox/src/sensors.rs)
- Camera recognition objects are the **physics shapes** of the rigid scene, labelled by entity name (`shape_objects(rigid, &own, labels)`). It publishes id, pixels, bbox, position and size. — [bin/gearbox/src/sensors.rs:758-797](bin/gearbox/src/sensors.rs), [bin/gearbox/src/sensor_generic.rs:81-100](bin/gearbox/src/sensor_generic.rs)
- Bevy colour cameras for `optimized`/`full` render on demand and are read back. The lean ones get `NoVegetation`. — [bin/gearbox/src/sensor_cameras.rs:1-8,153-162](bin/gearbox/src/sensor_cameras.rs)
- Light sensor:
  - publishes `[irradiance, direct, sky, visible, value]` — [sensor_generic.rs:209-215](bin/gearbox/src/sensor_generic.rs)
  - takes "the brightest directional light as the sun", split by cloud cover — [sensor_generic.rs:376-392,400-417](bin/gearbox/src/sensor_generic.rs)
  - casts its rays on the CPU "against the live rigid scene" — [sensor_generic.rs:1-8](bin/gearbox/src/sensor_generic.rs)
- `assets/world/sensor_suite.usda` shows how to author one link per sensor kind (`GearboxLinkAPI`, `gearbox:link:role = "sensor"`, `gearbox:sensor:kind`). — [bin/gearbox/assets/world/sensor_suite.usda:1-60](bin/gearbox/assets/world/sensor_suite.usda)

**Actuators**
- Actuator devices follow Webots: motors and brakes on joints, propellers, belts, **connectors** (docking faces that can lock to a peer, or to the world when `body` is None), copters and aerodynamic drag in the weather wind. — [bin/gearbox/src/devices.rs:1-10](bin/gearbox/src/devices.rs), [bin/gearbox/src/physics/backend.rs:838-865](bin/gearbox/src/physics/backend.rs), [specs/MACHINE_SPEC.md:571,623-624](specs/MACHINE_SPEC.md)
- Work controllers are bookkeeping only. `builtin:section_control` integrates TotalArea and distance. `builtin:rate_control` drains bin `ActualVolumeContent` at l/ha × ha/s. Neither leaves any mark in the world. — [bin/gearbox/src/services.rs:884-970](bin/gearbox/src/services.rs)
- Soil tools: a machine whose `gearbox:machine:kind` names a disc harrow stamps the work map where its disc colliders touch the ground. — [bin/gearbox/src/controller/soil_work.rs:1-11](bin/gearbox/src/controller/soil_work.rs), [crates/gearbox-fields/src/tillage/mod.rs:21-44](crates/gearbox-fields/src/tillage/mod.rs)

**Driving and control**
- Per-machine topics:
  - `/machines/<id>/{info, claim, release, session, cmd_vel, cmd, state, odom, links, tf, encoders, imu, gnss, turn_radius, tools, tools/attach, tools/detach}`;
  - `sensors/<link>`, `emit`, `actuate`, `actuators/<device>`, `description`, `joint_command`, `control`, `joint_state`.
  - Source: [crates/gearbox-api/src/topics.rs:19-68](crates/gearbox-api/src/topics.rs)
- Rust client methods: claim, release, cmd_vel, cmd_twist, command, state, odom, attach, detach, tools, links, description, acquire_lease, release_lease, reset_control, joint_command, joint_state, tf. — [crates/gearbox-api/src/client.rs:332-452](crates/gearbox-api/src/client.rs)
- The Python client has the same API, plus `set_fields` and `events`. — [examples/python/gearbox_client.py:482-777](examples/python/gearbox_client.py)
- CLI: `machine list|info|state|move|stop|drive|cmd|controllers|links|tf|set-value|tools`. — [docs/CLI.md:94-111](docs/CLI.md)
- Example drive: `gearbox machine move <id> --forward 3 --for 20s` after `gearbox scene play`. — memory note project_launch_from_shell (hint)
- Controller types:
  - drive: `builtin:ackermann_cmd_vel`, `diff_drive_cmd_vel`, `tracked_cmd_vel`, `copter`;
  - service: `hitch`, `pto`, `hydraulic_valve`, `joint_position`, `joint_velocity`, `brake`, `trailer_steer`;
  - work: `section_control`, `rate_control`.
  - Instance names `drive`, `arm` and `implement` are recognised.
  - Source: [specs/CONTROLLER_SPEC.md:71-105](specs/CONTROLLER_SPEC.md); `joint_position` semantics at [CONTROLLER_SPEC.md:429](specs/CONTROLLER_SPEC.md)

**Arm and gantry support**
- Supported joint types: `PhysicsRevoluteJoint`, `PhysicsPrismaticJoint` and D6 (`PhysicsJoint`), each with limits and drives. — [specs/MACHINE_SPEC.md:298-331](specs/MACHINE_SPEC.md)
- The robot contract lists `joint.revolute`/`prismatic` (exact, Isaac-verified), `joint.d6`, `joint.world_anchor` (exact, Isaac-verified), position/velocity/effort commands and hold/free/reset/pause, with evidence including `e02_arm`. — [specs/ROBOT_CONTRACT.md:423-453](specs/ROBOT_CONTRACT.md)
- Fixtures: [tests/fixtures/robot_contract/e01_arm.usda](tests/fixtures/robot_contract/e01_arm.usda), [e01_arm_gearbox.usda](tests/fixtures/robot_contract/e01_arm_gearbox.usda), [schedules/e02_arm.json](tests/fixtures/robot_contract/schedules/e02_arm.json)
- The `standard` profile is a bare articulation with no controllers. — [specs/ROBOT_CONTRACT.md:48](specs/ROBOT_CONTRACT.md), [specs/MACHINE_SPEC.md:114](specs/MACHINE_SPEC.md)

### Inferences
- A **gantry** fits cleanly: a world-anchored base plus prismatic X and Y joints, a prismatic Z joint and a wrist revolute, each with `builtin:joint_position` or direct `joint_command` leases. A **mobile manipulator** can be adapted from Perseverance's arm, or the Robotti or Ekobot tool joints. Both are expressible today as USD.
- Grasping fruit would need either a Connector device on the end effector plus a fruit body carrying a passive connector, or a scripted "harvest on contact" rule like the bale rule (Q7). There is no gripper or contact-grasp controller.
- **Perception gap.** Depth, LiDAR, recognition, distance and light rays all see only physics colliders. GPU-procedural plants have no colliders, so a robot camera's depth, LiDAR and recognition cannot see any plant drawn by `gearbox-fields`. Only a `full` Bevy colour render shows them, and it has no per-plant labels. Plants that must be sensed need to be physics shapes, or a new label/instance-ID render pass is needed.
- **Glass and the light sensor.** A glazed dome made of colliders would block light-sensor rays and LiDAR. That is wrong for glass, so glazing likely needs `physics:collisionEnabled = false` ([specs/MACHINE_SPEC.md:271](specs/MACHINE_SPEC.md)) or a sensor-transparent material flag. That flag does not exist.

### Gaps
- I did not check whether the light sensor counts point or spot grow lights. The code reads only the brightest DirectionalLight (sensor_generic.rs:379-384), so grow lights are probably invisible to it.
- I did not check whether `describe-machines` flags any of these packages as unusable on Mars; the only gravity note is PLANET_WORLDS.md:31.
- No segmentation or instance-ID camera channel exists: the channels are colour, depth and shape-index for recognition (sensors.rs:297-302).

## Q7. API: how would plant state (height, health, ripeness) be exposed to clients? Is there a pattern for publishing world state on topics?

### Takeaway
World state reaches clients by three existing patterns:
- **Host events**: `/gearbox/scene/events` publishes `SceneEvent { x, y, z, top_y, kind, props }`, with kinds loaded, pose, **harvested**, removed, machine_ready, attached, detached and machine_rejected.
- **Que/ans listing**: `/gearbox/scene/list` returns `SceneObject { x, y, z, yaw, kind, props }`.
- **Per-machine streams** of `Measurement`.

A working **harvest precedent** already exists: loaded props named `bale_*` are despawned when a dynamic machine body touches them, and a `harvested` event is published with `bale_id`.

Plant state would fit as a new event kind, a new host topic (e.g. a que/ans "plants" list), or props on `SceneObject`. Each requires adding a server in `HostBus` and a datapod wire type.

### Cited Findings
**Host topics and types**
- Host topics: `/gearbox/info`, `/scene/clock`, `/scene/clock/state`, `/scene/clear`, `/scene/list`, `/scene/events`, `/usd/load`, `/usd/delete`, `/marker/set|delete`, `/select`, `/machines/list`, `/access/grant`, `/fields/layout`. Exact-match peerbus topics, so ids ride in payloads. — [crates/gearbox-api/src/topics.rs:1-17](crates/gearbox-api/src/topics.rs)
- `HostBus` creates one `req_server`/`que_server`/`publish` per topic and serves each with `serve_*` closures. `publish_event(&SceneEvent)` exists. — [crates/gearbox-api/src/host.rs:94-150,209-271](crates/gearbox-api/src/host.rs)
- Event kinds 0–7 with names — [crates/gearbox-api/src/wire/host.rs:122-146](crates/gearbox-api/src/wire/host.rs)
- `SceneEvent` is datapod `gearbox.scene_event.v1` — [wire/host.rs:217-226](crates/gearbox-api/src/wire/host.rs)
- `SceneObject` is `gearbox.scene_object.v1` with kind ∈ any, machine, prop, marker, terrain — [wire/host.rs:93-120,199-208](crates/gearbox-api/src/wire/host.rs)
- `FieldsLayout` carries the layout JSON in a byte vec — [wire/host.rs:455-459](crates/gearbox-api/src/wire/host.rs)

**Precedents for publishing world state**
- `publish_loaded_usd_poses` publishes a settled prop's pose once. — [bin/gearbox/src/world.rs:960-1000](bin/gearbox/src/world.rs)
- `harvest_bales_on_machine_contact`:
  - detects active contacts between a `UsdLoad[bale_*]` prop collider and any non-prop dynamic body;
  - removes its physics, despawns it, and publishes `HARVESTED` with props `bale_id`.
  - Source: [world.rs:1002-1058](bin/gearbox/src/world.rs). Documented at [docs/SCHEMA.md:111-113](docs/SCHEMA.md)

**Machine-side state**
- Machine-side state uses `MachineState` (`gearbox.machine_state.v1`) and `Measurement` (`gearbox.measurement.v1`). Measurement kinds include LIGHT (7), RECOGNITION (12), MOTOR, PROPELLER and BELT. — [crates/gearbox-api/src/wire/machine.rs:170,379,401-437](crates/gearbox-api/src/wire/machine.rs)
- Per-link named values exist (`machine set-value LINK NAME VALUE`; ISOBUS-like `ActualWorkState`, `TotalArea`, `ActualVolumeContent`). — [bin/gearbox/src/services.rs:884-970](bin/gearbox/src/services.rs), [docs/CLI.md:110](docs/CLI.md)

**Tooling and clients**
- The CLI `api topics|call|que|schema` handles raw access, and `gearbox subscribe <TOPIC>` streams any topic. — [docs/CLI.md:113-146](docs/CLI.md)
- JSON payloads go through `gearbox_api::wire::json`. — [crates/gearbox-api/src/wire/json.rs](crates/gearbox-api/src/wire/json.rs); memory note project_cli_layout (hint)
- A fake host makes GPU-less testing possible: `cargo run -p gearbox-api --no-default-features --example fake_host`. — [crates/gearbox-api/examples/fake_host.rs](crates/gearbox-api/examples/fake_host.rs); memory note project_agentio_transport

### Inferences
- The lowest-friction path for plant state:
  - represent each plant as an entity with a stable id;
  - serve a new que/ans topic (e.g. `/gearbox/plants/list`) returning a datapod `PlantState { id, x, y, z, species, stage, height_m, health, ripeness, props }`;
  - publish growth-stage and harvest transitions on `/gearbox/scene/events` as new `event_kind`s (e.g. `PLANTED`, `STAGE`, `HARVESTED` reused);
  - add `sow`/`harvest` requests as req/res topics.
- The bale contact harvest is a ready template for "robot end-effector touches ripe fruit → fruit removed → event".
- The `Props` string map on every wire type is a schema-free escape hatch. Early iterations could carry plant attributes in props of `SceneObject` and `SceneEvent` without new datapod types.

### Gaps
- No plant, crop, field-state or environment (temperature, humidity, CO₂) topic or type exists anywhere in gearbox-api.
- The datapod wire-type naming is `gearbox.<name>.v1`, but the memory says "one schema, no versions" (ROBOT_CONTRACT); how new types should be named was not checked.

## Q8. Gaps: what is missing to (1) render a Mars greenhouse interior, (2) grow plants over time with environmental inputs, (3) let robots sense and act on plants?

### Takeaway
1. **Rendering** is mostly there: world USD, UsdLux lights, transmissive or blended glass, the Mars sky and ground. What is missing is a flat building pad, an interior and exterior microclimate, plants on non-terrain surfaces, and checks of glass shadows and exposure.
2. **Growth** is entirely missing: no plant entity, no state, no growth model, no environment variables, no growth clock, no sow or harvest logic. The existing vegetation is stateless by design.
3. **Sensing and acting** has the robots, arms, joints, leases, cameras, light sensor and a bale-harvest precedent. Plants, however, are invisible to depth, LiDAR and recognition, which see colliders only. There is also no gripper, sowing tool or plant API.

### Cited Findings
**(1) Rendering a Mars greenhouse interior**
- Already there:
  - Mars launch, sky, sun and ground (Q1);
  - world USD loading with static colliders — [bin/gearbox/src/physics/mod.rs:145-197](bin/gearbox/src/physics/mod.rs), [specs/MACHINE_SPEC.md:250-270](specs/MACHINE_SPEC.md);
  - UsdLux to Bevy lights — [usd_bevy/src/route/light.rs:1-13](/home/bresilla/data/code/OUSD/bevy_openusd/crates/usd_bevy/src/route/light.rs);
  - glass via opacity and transmission — [usd_bevy/src/route/material.rs:420-465](/home/bresilla/data/code/OUSD/bevy_openusd/crates/usd_bevy/src/route/material.rs);
  - CPU clustering serving USD point and spot lights — [bin/gearbox/src/app.rs:120-128](bin/gearbox/src/app.rs).
- Missing:
  - **Level pad.** Mars home is flat for only about 6 m — [crates/gearbox-globe/src/terrain.rs:49](crates/gearbox-globe/src/terrain.rs). Boulders are excluded for only 8 m — [bin/gearbox/src/planet_rocks.rs:215](bin/gearbox/src/planet_rocks.rs). Neither can be configured per structure.
  - **No greenhouse asset** in `bin/gearbox/assets` or `machines/` (directory listings).
  - **Vegetation sources.** Only meadow or weed Poly Haven clumps exist (7 packs) — [crates/gearbox-fields/src/clumps/mod.rs:58-64](crates/gearbox-fields/src/clumps/mod.rs). There are no crop models (lettuce, tomato, wheat, potato), and the packer script is missing from the tree.
  - **Placement on terrain only.** Plants can only be placed on the procedural cover terrain heightmap — [crates/gearbox-fields/src/host.rs:8-26](crates/gearbox-fields/src/host.rs). On Mars all grass densities are 0 — [bin/gearbox/src/environment.rs:41-49](bin/gearbox/src/environment.rs).
  - **No vegetation shadows** — [crates/gearbox-fields/src/render.rs:198](crates/gearbox-fields/src/render.rs)
  - **Global wind.** One wind applies indoors and outdoors — [crates/gearbox-fields/src/host.rs:36-49](crates/gearbox-fields/src/host.rs)
  - **Fixed exposure.** EV100 is fixed per planet — [bin/gearbox/src/environment.rs:33](bin/gearbox/src/environment.rs). There is no automatic exposure for walking from outdoor sun into a lit interior.
  - **Biomes and dust motes** are disabled on Mars — [bin/gearbox/src/app.rs:108-110](bin/gearbox/src/app.rs)

**(2) Growing plants with environmental inputs**
- **No plant concept.** Vegetation instances are a hash of the instance index, re-derived per frame — [crates/gearbox-fields/src/clumps/shaders/clumps.wgsl:143-200](crates/gearbox-fields/src/clumps/shaders/clumps.wgsl). `VegetationLayer` has no state fields — [crates/gearbox-fields/src/profile.rs:49-80](crates/gearbox-fields/src/profile.rs). The `bevy_biomes` `Plants` type is a static recipe with `height_m` ranges — [crates/bevy_biomes/src/layer.rs:12-21](crates/bevy_biomes/src/layer.rs).
- **No environment variables.** Temperature, humidity, CO₂, soil moisture, nutrients and PAR are not in `WeatherSettings` — [crates/bevy_weather/src/lib.rs:26-50](crates/bevy_weather/src/lib.rs). The only light quantity is the sun plus `SkyLight` — [lib.rs:171-205](crates/bevy_weather/src/lib.rs), [bin/gearbox/src/sensor_generic.rs:376-417](bin/gearbox/src/sensor_generic.rs). Grow lights are not counted.
- **No time scale.** [crates/gearbox-cli/src/cli/scene.rs:76-78](crates/gearbox-cli/src/cli/scene.rs). There is no automatic diurnal sun and no Mars sol calendar — [crates/bevy_weather/src/calendar.rs:3-40](crates/bevy_weather/src/calendar.rs), [bin/gearbox/src/environment.rs:24-27](bin/gearbox/src/environment.rs).
- **Only per-texel persistence.** The closest analogues are the permanent tillage work map — [crates/gearbox-fields/src/tillage/mod.rs:1-11](crates/gearbox-fields/src/tillage/mod.rs) — and the decaying wheel map — [crates/gearbox-fields/src/profile.rs:23-47](crates/gearbox-fields/src/profile.rs).

**(3) Robots sensing and acting on plants**
- **Sensing.** Depth always comes from the Molla collision trace — [bin/gearbox/src/links.rs:287-298](bin/gearbox/src/links.rs). Recognition uses physics shapes — [bin/gearbox/src/sensors.rs:790-797](bin/gearbox/src/sensors.rs). Distance, light and radar rays hit the "live rigid scene" — [bin/gearbox/src/sensor_generic.rs:7-8](bin/gearbox/src/sensor_generic.rs). Procedural plants are therefore invisible to everything except `full` colour renders. Optimized renders exclude vegetation — [bin/gearbox/src/sensor_cameras.rs:153-162](bin/gearbox/src/sensor_cameras.rs).
- **Acting.** The world-changing actions that exist are:
  - a disc harrow work map — [crates/gearbox-fields/src/tillage/mod.rs:21-44](crates/gearbox-fields/src/tillage/mod.rs);
  - bale contact-harvest — [bin/gearbox/src/world.rs:1002-1058](bin/gearbox/src/world.rs);
  - connectors — [bin/gearbox/src/physics/backend.rs:838-865](bin/gearbox/src/physics/backend.rs).
- **Not world-changing.** Section and rate control are bookkeeping and never mark the ground — [bin/gearbox/src/services.rs:884-970](bin/gearbox/src/services.rs).
- **Missing actions.** There is no sow, transplant, water, spray-deposit, prune or pick action, and no plant-targeted API.
- **Arms exist.** Arm and gantry kinematics and control are available: Perseverance has a 5-DOF arm plus drill, joint_position/joint_command leases are Isaac-verified, and there is world_anchor — [specs/ROBOT_CONTRACT.md:423-453](specs/ROBOT_CONTRACT.md). No greenhouse-specific robot (gantry, rail-cart, harvesting arm) is in `machines/` (describe-machines output).

### Inferences
- **Minimum new pieces, in dependency order:**
  1. A greenhouse world USD with a slab foundation, glazing (collisions off or sensor-transparent), beds and grow lights.
  2. A `plants` module: an ECS `Plant { id, species, planted_at, stage, biomass, height, health, ripeness }` with a growth system on a scaled growth clock. Its inputs would be a light integral (sun through the glazing plus grow lights), temperature and water from a new greenhouse `Climate` resource.
  3. Rendering per plant: Poly Haven-style crop assets as instanced meshes with per-instance scale and stage. Options are a per-plant storage buffer in `gearbox-fields` (layout can follow `blades.rs` records), or plain Bevy entities, since crops number thousands rather than millions.
  4. Physics proxies: simple static or kinematic colliders, labelled by plant id, so depth, LiDAR and recognition see the plants. The sensor rig re-snapshots on scene revision (sensors.rs:1-8), so plant colliders should change rarely, for example on stage transitions only.
  5. API: a plants list, events and sow/harvest req/res topics modelled on `HostBus` + `SceneEvent`, plus a gripper/connector or contact-harvest rule modelled on `harvest_bales_on_machine_contact`.
  6. Clock: a growth-acceleration factor, plus an optional Mars sol sun driver feeding `WeatherSettings.sun_position`.

### Gaps
- I did not measure how many extra static colliders the Molla sensor snapshot and contact solver tolerate. That decides whether every plant can have a collider or only clusters.
- Not covered, because no relevant code exists: radiation shielding, pressure differential and greenhouse thermal balance.
- I did not inspect the mara host panes for adding a greenhouse or plant pane beyond the pane list: machine, scene, view, log, environment, capture, shortcuts — [bin/gearbox/src/host/mod.rs:47-53](bin/gearbox/src/host/mod.rs).
