# Earth, Mars and Moon

Earth, Mars and Moon are launch-selectable full procedural worlds. Native ground/orbit views, global travel and Perseverance driving were verified on the release build recorded below.

## Launch contract

`gearbox run --planet earth|mars|moon` selects one procedural planet for an instance. Earth remains the default. `GEARBOX_PLANET` supplies the same selection for direct launches; `gearbox launch --planet mars` overrides it. `--world` remains a USD scene argument, not the planet selector.

The repository launch recipe forwards the selection:

```sh
oslo make run
oslo make run --args='--planet mars --load assets/machines/usd/perseverance_nasa.usdz'
oslo make run --args='--planet moon --load assets/machines/usd/perseverance_nasa.usdz'
```

Use `--args='...'` with the equals sign: Oslo treats a separate dash-prefixed value as another flag. Paths are resolved from the Gearbox working directory; the machine packages are in the `machines` submodule.

Different instances can select different planets. Selection is fixed for each process; changing the entire planet at runtime is not part of the launch contract. Perseverance on the Moon is a simulation scenario, not a reconstruction of a lunar mission.

## Environment contract

| | Earth | Mars | Moon |
|---|---|---|---|
| Reference geometry | Existing WGS84 | 3389.5 km mean-radius sphere | 1737.4 km mean-radius sphere |
| Gravity | Existing 9.81 m/s² | 3.711 m/s² | 1.62 m/s² |
| Local surface | Existing fields | Rust-coloured regolith, dunes, rocks and craters | Grey regolith, ejecta, rocks and craters |
| Sky | Existing weather | Scattered CO2 and dust (Bevy `Atmosphere`, Mars preset), dust optical depth `dust_tau`, no clouds | Black, no atmospheric haze or clouds |
| Vegetation | Existing field layout | None | None |

Terrain is procedural, not a surveyed reconstruction of Jezero or Apollo 11. Launch coordinates are reference locations. Planetary frames, streamed ground and collision heights use the same body. Imported machine USD scenes must not replace Mars/Moon gravity with their embedded Earth gravity.

The procedural globe includes multi-scale impact basins and regional albedo variation. Mars has dark basalt-like provinces and polar ice colouring; the Moon has dark mare-like regions and lighter highlands. These are generated features, not maps of named real locations. Local and globe relief share the same crater/noise equations. Hardware probes currently agree within 6.3 cm on Mars and 4.7 cm on the Moon over 384 samples per body; this is sampled evidence, not a global error bound.

The local surface joins the globe spatially. Its mask uses the installed terrain's floating-origin render frame and actual rasterized positions, with a two-metre overlap at the detailed mesh perimeter. Barren worlds do not use Earth's radial backdrop, altitude cutoffs or screen-door fades. Local and global materials share regional geology colouring; fine local grains fade with pixel footprint. Local tangent heights solve the radial terrain intersection, and the globe uses explicitly bilinear R32Float heights and floating-origin-aware morph distances. Barren globe leaves resolve sub-metre nominal cells; local planetary normals come from height differences rather than the terrestrial normal decoder. Native captures show continuous relief without the previous sky-coloured perimeter cracks. Fine detail still changes with LOD; this is not a pixel-identical local/global mesh.

The globe selector reserves space for unvisited siblings and face roots, retaining coarser resident parents rather than dropping terrain when its render budget is exhausted. Conservative projected bounds skip barren tiles wholly covered by installed local terrain. All final Earth/Mars/Moon captures settled with zero dropped tiles and zero atlas allocation failures. Near-surface budget coarsening is recorded, not treated as full refinement.

Terrain replacement preserves the retiring ground's surface children until its successor is ready. A regression test checks that lifecycle; native intermediate travel captures confirm that the ground no longer disappears during the handover. Coverage waits for local meshes and assigned surface materials, and travel verification waits for the visible handover as well as globe selection.

Both barren worlds now stream deterministic geodetic boulders with matching triangle-mesh wheel colliders. Exact native ray/visible-triangle tests and cross-site placement tests pass; the Mars native capture shows embedded rocks and regolith without plough stripes.

Native evidence includes Mars/Moon ground and orbital views and short straight-line rover motion from TF streams (1.77 m on Mars, 1.72 m on Moon, stable base height). Both worlds completed antipode, remote daylight and home-return travel without a panic. The antipode site-distance fix and renderer sparse-buffer fix passed their regressions. `vendor/bevy_render/GEARBOX_PATCH.md` records the focused fix; `oslo make test-render-pages` checks its page-boundary bookkeeping. A separate startup slab-allocator warning remains visible in logs, including Earth; no zero-warning or long-duration vehicle-calibration claim is made.

`oslo make test-planet-gravity` runs the actual physics world and USD scene-import systems in separate body-selected processes. After importing two gravity scenes, measured one-second accelerations are 9.81, 3.711 and 1.62 m/s² for Earth, Mars and Moon respectively. This is not a rover drop or suspension-calibration test.

Physical references: [NASA Mars facts](https://science.nasa.gov/mars/facts/), [ESA Mars facts](https://www.esa.int/Science_Exploration/Space_Science/Mars_Express/Facts_about_Mars), [NASA Moon fact sheet](https://nssdc.gsfc.nasa.gov/planetary/factsheet/moonfact.html). Colours, procedural geology and lighting are visual approximations, not calibrated radiometry.

## Mars time, sky and the pad

- **Calendar.** The sun follows Mars24 (Allison and McEwen): solar longitude,
  declination, equation of centre and heliocentric distance, on a 88 775.244 s
  sol and a 668.5991-sol year. Sunlight is 586.2/1361 of Earth's (about
  56 000 lux), scaled by the distance of the day.
- **World clock.** The sun, the weather, crops and greenhouse air run on world
  time, `rate` × the physics clock; machines run on the physics clock.
  `gearbox scene speed 600` sets the rate (up to 10⁶), `gearbox scene skip
  30sol` jumps ahead in slices of a twelfth of a day per frame, and `gearbox
  scene status` shows the day and hour.
- **Sky.** Bevy's raymarched `Atmosphere` with the Mars scattering medium
  (Schneegans et al. 2024) over 120 km of air, lighting the scene through
  `AtmosphereEnvironmentMapLight`. `dust_tau` (default 0.4, up to 12) thickens
  the dust over the preset's 0.146: butterscotch by day, a
  red-brown storm at 4. `gearbox scene weather dust_tau=4` sets it live.
- **Pad.** Off Earth, home is a prepared plain, level within 320 m and back
  to full relief at 480 m; no rocks stand on it, so a base and its landing
  zone can be laid on it.
- **The Mars base.** `--world bin/gearbox/assets/world/mars_base.usda` loads
  a base that grows its food, its structures referenced from
  `assets/mars_base/modules/`; `gearbox spawn machine
  bin/gearbox/assets/world/mars_base_fleet.usda --machine base` brings its
  tending robots and rovers. See `specs/GREENHOUSE_SPEC.md`.

## Acceptance gates

- [x] Body-aware metre-scale coordinate round trips and global travel tests.
- [x] Deterministic distinct crater-bearing local height functions and unchanged Earth unit tests.
- [x] Initial whole-workspace type check.
- [x] Expanded CLI, environment and sky tests.
- [x] Production GPU globe-height shader parity against CPU terrain samples.
- [x] Coherent distant globe relief and local-to-orbit transitions, including opposite-side travel.
- [x] Native screenshots of Earth, Mars and Moon, near ground and from orbit.
- [x] Perseverance loaded, settled and driven on Mars and Moon with valid TF and ground contact.
- [x] Planetary gravity verified in the runtime physics/import systems after USD gravity scenes.
- [x] Earth default launch and explicit Earth regression check.

`oslo make test-planets` runs the focused tests. `oslo make test-planet-gpu` checks production height parity and the production bilinear height sampler against 288 fractional, integer and border probes across two atlas layers. Continuation notes and current logs are in `.work/planet_worlds/`.

## Verification record

Release SHA256: `d6910c6fa22b4b599691dfa8aed5ca0e9ea051d1afa03cdd7d9646ca48433b5d`.
The canonical rover remained unchanged: `f11f6c7041472e059840264dabfffbe9de824bb43ee59ed1e407a454367fbc5f`.

Evidence under `.work/planet_worlds/`:

- `test-overlap.log`, `check-overlap.log`, `build-overlap.log`: focused tests, full workspace check, release build.
- `test-render-mask-gpu.log`: hardware height parity and production sampler tests.
- `{mars,moon}-overlap-ranges.json`: ground, 1200 m, 6500 m and orbital captures with selector counters.
- `mars-overlap-low-travel.json`, `moon-overlap-travel.json`: opposite-side/daylight/home requests, actual poses, visible handovers and captures.
- `{mars,moon}-overlap-{before,after}.jsonl`, `*-overlap-driven.png`: actual rover TF motion and rendered contact views.
- `earth-{default,explicit}-overlap-ranges.json`: both launch modes, ground and orbit.
- `acceptance.json`, `audit.log`: `make -C .work/planet_worlds audit` validates the recorded artifacts, binary/rover fingerprints, launch modes, tile counts and TF motion. Visual review was performed separately on the actual screenshots.
