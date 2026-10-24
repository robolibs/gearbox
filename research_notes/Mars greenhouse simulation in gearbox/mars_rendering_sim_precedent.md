# Mars rendering, greenhouse visuals, and game / simulator precedent for a gearbox Mars greenhouse site (as of October 2026)

Context checked locally: gearbox's workspace pins `bevy = "0.19"` (resolved to 0.19.1 in the cargo registry), and `crates/gearbox-planet/src/planet/mod.rs` builds every planet atmosphere from `ScatteringMedium::earth(256, 256)` with an overridden Rayleigh term, `with_density_multiplier(..)` and a hard-coded `ground_albedo: Vec3::splat(0.3)`. A Mars site therefore needs a different medium, not only different numbers.

## 1. Mars appearance physics (sky colour, sunsets, sun, irradiance, dust, storms, dust devils, frost, regolith)

### Takeaway
The Mars sky is coloured by suspended dust, not by gas. The dust particles are about 1–2 µm across and contain iron oxide, which absorbs blue light. That gives a butterscotch or yellow-brown sky in daytime and a blue glow tightly around the sun, which is strongest at sunset. CO2 Rayleigh scattering is negligible by comparison. Sunlight at the top of the atmosphere averages 586 W/m² (43% of Earth's) and swings by about ±19% with Mars's eccentric orbit. Dust optical depth (tau) drives everything visible: clear about 0.15–0.5, dusty about 1, the 2007 storm about 5, and the 2018 storm 10.8.

### Cited Findings
**Orbit, irradiance, bulk properties**
- Solar irradiance at Mars is 586.2 W/m² against Earth's 1361.0 W/m², a ratio of 0.431. Bond albedo is 0.250 and geometric albedo 0.170. Semi-major axis 227.956×10⁶ km, perihelion 206.650×10⁶ km, aphelion 249.261×10⁶ km. Surface pressure is 6.36 mb (seasonal range 4.0–8.7 mb) with an 11.0 km scale height. The atmosphere is 95.1% CO2, 2.59% N2 and 1.94% Ar by volume. Mean surface temperature is about 214 K, with a diurnal range of 184–242 K at Viking 1. A solar day is 24.6597 h. — [NASA NSSDC Mars Fact Sheet](https://nssdc.gsfc.nasa.gov/planetary/factsheet/marsfact.html)
- Appelbaum & Flood (NASA Lewis) model the direct, diffuse and global insolation on the Martian surface from measured tau (Viking camera sun-diode images) and multiple-scattering calculations. The direct beam follows Beer's law. This is the standard engineering reference for surface irradiance as a function of tau. — [NASA TM-102299 record](https://ntrs.nasa.gov/citations/19890018252); [1991 update TM-105216](https://ntrs.nasa.gov/citations/19910005804)

**Sky colour and blue sunset**
- Curiosity imaged its first colour sunset on sol 956 (15 April 2015) in Gale Crater. Mark Lemmon explained that dust particles of a particular size let blue light through more efficiently. The blue light scattered by that dust stays close to the sun, while the rest of the sky looks yellow to orange. Mastcam is a little less sensitive to blue than human eyes. — [NASA PIA19400 / PDS](https://pds-rings.seti.org/press_releases/pages/PIA19xxx/PIA19400.html); [Spaceflight Now](https://spaceflightnow.com/2015/05/11/curiosity-rover-sees-blue-on-the-red-planet/)
- Mie and Debye-series modelling (Ehlers et al., Applied Optics) shows that the bluish glow around the sun comes from strong near-forward scattering of blue light by the dust. A simple Rayleigh-style model cannot reproduce it. — [WUSTL record "Blue moons and Martian sunsets"](https://profiles.wustl.edu/en/publications/blue-moons-and-martian-sunsets/)
- Bell, Savransky & Wolff (2006) derived sky chromaticity from calibrated MER Pancam data. The sky ranges from "dark yellowish brown" at high dust opacity to "bluish-black" or "black" for dust-free air, and the paper gives a functional relation between sky colour and optical depth. — [Bell et al. 2006, JGR E12S05](https://asu.elsevierpure.com/en/publications/chromaticity-of-the-martian-sky-as-observed-by-the-mars-explorati)
- A Monte Carlo Mars sky model renders tau = 0.4, 0.8 and 1.2 with CRISM-derived aerosols. It uses dust single-scattering albedo 0.922 at 615 nm, 0.797 at 551 nm and 0.690 at 450 nm. Most of the sky comes out "strawberry or butterscotch", turning neutral or bluish only near the sun. As dust thickens the overall light turns a dull red and the faint sun looks bluer. — [Steve Albers, "The Sky on Mars"](https://stevealbers.net/albers/allsky/mars.html)
- Schneegans et al. 2024 model the dust as feldspar with 3% hematite (after Ehlers et al.). Without hematite the simulated sky is grey. Above about 6% hematite it becomes unrealistically red. Real photos vary between almost no blue glow and a very strong one, and hematite content cannot explain that variation. — [Schneegans et al., CGF 43(2) 2024, doi:10.1111/cgf.15010](https://cgvr.cs.uni-bremen.de/papers/cgf24/v43i2_15_15010.pdf)

**Dust particle size and optical depth**
- MER solar imaging over five Mars years records local, regional and planet-encircling storms. Dust effective radius is about 1 µm in northern summer and about 2 µm at the onset of dust lifting. Cirriform clouds appear at Opportunity over Ls 20–136° (peaks near Ls 50° and 115°). Water-ice haze and early-season CO2 clouds are possible. — [Lemmon et al. 2015, Icarus 251 (arXiv 1403.4234)](https://arxiv.org/abs/1403.4234)
- Published effective radii include r_eff ≈ 1.6 µm (Tomasko et al. 1999) and 1.5 µm. Schneegans et al. fit a bimodal log-normal: r_eff,small = 0.04 µm, v_eff = 0.1; r_eff,large = 1.4 µm, v_eff = 0.35; mode ratio 1000. — [Schneegans et al. 2024](https://cgvr.cs.uni-bremen.de/papers/cgf24/v43i2_15_15010.pdf)
- Opportunity measured tau ≈ 10.8 on 10 June 2018, a record for measurements on Mars and above the Viking landers' ~9 from a 1970s storm. The 2007 storm peaked at about 5–5.5. NASA described the 2018 storm as more diffuse and patchy than the Mariner 9 or MGS-era storms, which hid the surface completely. — [Planetary Society MER update, June 2018](https://www.planetary.org/articles/06-mer-update-opportunity-dust-storm-sleep); [JPL news](https://www.jpl.nasa.gov/news/opportunity-hunkers-down-during-dust-storm/)
- Montabone et al. (2015, Icarus) compiled an eight-year orbiter dust-opacity climatology covering Mars years 24–31 (TES, THEMIS, MCS). — [Open University record](https://oro.open.ac.uk/43368)

**Dust devils**
- On 25 January 2025 (sol 1,398), on the western rim of Jezero Crater, Perseverance's navcam recorded a dust devil about 65 m wide and at least 400 m tall moving about 1 m/s WNW. It absorbed a smaller devil about 5 m wide. The clip is 21 frames about 6 s apart. — [NASA/JPL PIA26528](https://www.jpl.nasa.gov/images/pia26528-perserverance-captures-crater-rim-dust-devil-eating-its-own/); [NASA news](https://www.nasa.gov/missions/mars-2020-perseverance/perseverance-rover/perseverance-rover-witnesses-one-martian-dust-devil-eating-another)
- Dust opacity combines with the insolation season to control dust-devil frequency at Spirit's site. — [Lemmon et al. 2015](https://arxiv.org/abs/1403.4234)

**Frost**
- Viking Lander 2 at Utopia Planitia imaged white frost patches near rocks, in the sampler trench and on darker ground. Lander temperatures suggest it formed overnight and partly vanished by day. Its composition (CO2, water or a mix) was uncertain. — [NASA PIA00530 "Frost on Utopia Planitia"](https://photojournal.jpl.nasa.gov/catalog/PIA00530)
- CaSSIS (TGO) found morning water frost on the calderas of Olympus, Arsia and Ascraeus Montes and Ceraunius Tholus in cold seasons. It lasts only a few hours around sunrise, is reported as about 0.01 mm thick, and comes from upslope transport of water vapour. — [Valantinas et al. 2024, Nature Geoscience 17:608–616](https://nomad.aeronomie.be/publications/225-evidence-for-transient-morning-water-frost-deposits-on-the-tharsis-volcanoes-of-mars); [Univ. Bern release](https://mediarelations.unibe.ch/media_releases/2024/media_releases_2024/first_detection_of_frost_on_the_solar_systems_tallest_volcanoes_on_mars/index_eng.html)

**Regolith albedo and colour**
- Spirit Pancam measured low-albedo dust-devil tracks at about 0.20 against dustier deposits at about 0.30 (Bell et al. 2004). — [CRISM multi-angle study, arXiv 1408.5301](https://arxiv.org/pdf/1408.5301)
- Average scene albedos along Opportunity's Meridiani traverse ran from about 0.11 to 0.30. — [AGU FM15 abstract](https://agu.confex.com/agu/fm15/webprogram/Paper81778.html)
- Pancam's dust reflectance reference spectrum comes from optically thick dust on the rover magnets (Madsen et al. 2009). — [USGS radiative-transfer modelling of dust-coated Pancam targets](https://www.usgs.gov/publications/radiative-transfer-modeling-dust-coated-pancam-calibration-target-materials-laboratory)

**Dust on surfaces (relevant to greenhouse glazing and solar panels)**
- Solar-array output on Mars landers and rovers typically drops about 0.2% per sol, with an observed range of 0.05–2% per sol. Dust devils and gusts periodically cleaned Spirit's and Opportunity's arrays. InSight apparently never got significant cleaning. Curiosity's UV sensors show a similar, slightly higher degradation rate. — [Lorenz et al. 2021, PSS (INTA record)](https://digital.inta.es/entities/publication/e629d870-297b-4f08-89ce-ee637569c063/full); [IP Paris record](https://researchportal.ip-paris.fr/en/publications/lander-and-rover-histories-of-dust-accumulation-on-and-removal-fr/)
- At Opportunity, dust deposition was 0.004 ± 0.001 surface optical depth per sol per unit atmospheric tau, with a semiannual dust cycle. — [Göttingen record](https://grodev.d.sub.uni-goettingen.de/entities/publication/bc63f9ac-e609-4af8-80ee-34dc19736f79)

### Inferences
- **Sun angular size.** The sun's diameter (1.3927×10⁶ km) over the fact-sheet distances gives 0.00674 rad (23.2′) at perihelion, 0.00611 rad (21.0′) at the mean and 0.00559 rad (19.2′) at aphelion. Bevy's `SunDisk::EARTH` uses 0.00930842 rad, so a Mars `SunDisk` should use `angular_size ≈ 0.00611`.
- **Top-of-atmosphere irradiance.** It scales with 1/r²: about 713 W/m² at perihelion, 586 W/m² mean and 490 W/m² at aphelion. Bevy uses `lux::RAW_SUNLIGHT = 130_000` for unfiltered Earth sunlight. The Mars equivalents are about 68,100 lux (perihelion), 56,000 lux (mean) and 46,800 lux (aphelion). This is about 0.43× Earth, so a Mars scene needs roughly 1.2 EV more exposure than the same Earth scene. Bevy's own example reuses `RAW_SUNLIGHT` for Mars.
- **Rendering consequence of the dust size.** At 1–2 µm the dust sits well above the Rayleigh regime. A single Henyey–Greenstein lobe with constant g cannot give both a butterscotch sky and a tight blue aureole. The phase function must be chromatic, with a narrower forward lobe at short wavelengths, as in the Schneegans/Bevy tabulated texture.
- **Ground albedo is a free parameter, and Bevy's preset is dark.** Bevy's Mars preset uses `ground_albedo` 0.1, at the dark end of the measured 0.11–0.30 range. A dusty site such as Gale or Jezero is closer to 0.2–0.3, and the planet-wide Bond albedo is 0.25. The multi-scattering contribution will be too dark if 0.1 is left unchanged.
- **Frost** belongs in a gearbox site as a short morning-only effect: a thin white layer in shadowed or high places that burns off within hours of sunrise. It should not be a persistent snow cover.

### Gaps
- No primary source found with a seasonal "typical clear-sky tau" table at a specific site, such as aphelion around 0.3–0.5 and perihelion around 1. Montabone 2015 and Lemmon 2015 contain it, but the full text was not retrieved. The 0.3–1 range is supported only indirectly (Albers renders tau 0.4–1.2; storms 5–10.8).
- No authoritative sRGB value for "Mars regolith colour" was found. Measured spectra exist (Madsen 2009 magnet dust, Bell 2006 chromaticity) but were not retrieved as numbers.
- No source found for frost observed by Curiosity at Gale. The search returned only Viking 2 and the Tharsis results.

## 2. Rendering techniques: physically based Mars atmospheres, published parameters, Bevy configuration, dust volumetrics, how others render Mars

### Takeaway
The best published real-time Mars sky is Schneegans et al. 2024. It extends Bruneton's precomputed scattering with Mie-theory dust (feldspar + 3% hematite, bimodal log-normal sizes, double-exponential density) and is open source in CosmoScout VR. Bevy imported those exact numbers: `ScatteringMedium::mars` and `Atmosphere::mars` ship in Bevy 0.19.1, which gearbox already uses. They need a small chromatic phase texture (`mars_mie_phase.ktx2`) that lives in Bevy's repo assets, not in the crate. The preset corresponds to clear-sky tau of about 0.15. Dust storms need a density multiplier, and probably ground-level volumetric fog, on top.

### Cited Findings
**Published Mars atmosphere parameter sets**
- **Schneegans et al. 2024, Table 1** (values in brackets are for [440, 550, 680] nm):
  - *Molecules.* Rayleigh phase with depolarisation δ = 0.09. Refractive index n = 1.00000337, adapted to Mars surface pressure and temperature. Density ρ(h) = e^(−h/8000 m).
  - *Dust.* Mie theory with feldspar + 3% hematite IoR and the bimodal radius distribution, N = 5×10⁹ m⁻³. β_sca = [20.93, 25.39, 30.67] and β_abs = [9.33, 5.25, 1.26]. The table prints the unit as "µm⁻¹", but these are ×10⁻⁶ m⁻¹ per Bevy's port. Density ρ(h) = 0.75·e^(1−e^(h/4000 m)) + 0.25·e^(1−e^(h/20000 m)).
  - *Earlier models in the same table.* Collienne 2013 used dust β_sca = 3, no absorption, ρ = e^(−h/3200 m) and HG g = 0.76, with molecules at ρ = e^(−h/11000 m). Costa 2021 used a double-HG phase with g1 = [0.67, 0.4, 0.03] and g2 = 0.094, β_sca = [12.65, 18.51, 23.46] and β_abs = [24.36, 12.90, 7.852], with ρ = e^(−h/11100 m). The authors found errors in Costa's published numbers: molecular coefficients off by more than 1000× and an implausible dust density of 2.8×10³⁵ m⁻³.
  - *Source:* [Schneegans et al. 2024](https://cgvr.cs.uni-bremen.de/papers/cgf24/v43i2_15_15010.pdf)
- **Validation and performance.** Schneegans et al. compared against Pathfinder IMP sky radiance at sun elevation 75° (Markiewicz 1999) and MER Pancam chromaticity at sun elevations 0°, 45° and 90°. Their model reproduces the sharp dust forward peak much better than Collienne or Costa. Precomputation takes "a couple of seconds" for the Martian dust. Runtime cost equals Bruneton's: a few texture lookups per pixel. LUTs are 1024×512 transmittance and 64×16 irradiance. The strong forward scattering produces "a very high dynamic range" sky that can over-expose. For dense atmospheres they propose path-tracing primary rays and using LUTs only for secondary rays. — [Schneegans et al. 2024](https://cgvr.cs.uni-bremen.de/papers/cgf24/v43i2_15_15010.pdf)
- Collienne et al. (DLR, 2013) adapted Bruneton's precomputed tables to Mars and validated them against Martian imagery. — [DLR elib 86477](https://elib.dlr.de/86477)
- Hillaire 2020 ("A Scalable and Production Ready Sky and Atmosphere Rendering Technique", EGSR) avoids high-dimensional LUTs. Its MIT-licensed reference implementation includes a ray-marched LUT path, a path tracer that matches PBRT/Mitsuba, and a Bruneton comparison. The README has no Mars preset or numbers. — [sebh/UnrealEngineSkyAtmosphere](https://github.com/sebh/UnrealEngineSkyAtmosphere); [EG paper PDF](https://diglib7.eg.org/bitstream/handle/10.1111/cgf14050/v39i4pp013-022.pdf)
- Unreal's SkyAtmosphere (Hillaire's model) exposes the same knobs: Rayleigh/Mie scattering, scale, exponential falloff, Mie absorption, Mie anisotropy, planet radius and atmosphere height. The default Rayleigh scale height is 8 km and the slider runs 0.8–80 km. No official Mars preset was found. Hillaire's talk notes the sky updates every frame, so it can be tweaked to Mars live. — [Unreal SkyAtmosphere docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/sky-atmosphere-component-in-unreal-engine); [Hillaire SIGGRAPH 2020 slides](https://blog.selfshadow.com/publications/s2020-shading-course/hillaire/s2020_pbs_hillaire_slides.pdf)

**Bevy: what exists, by version**
- Bevy 0.18 made the procedural atmosphere tint the sunlight that reaches scene objects (orange or red near the horizon, working with volumetric fog). It also added the `ScatteringMedium` asset, built from `ScatteringTerm { absorption, scattering, falloff, phase }`. Falloff options include `Exponential` and `Tent`; phase options include `Rayleigh`, `Mie { asymmetry }` and `Isotropic`. The release notes list "non-earth-like planets" as a use case. — [Bevy 0.18 release notes](https://bevy.org/news/bevy-0-18/)
- Bevy 0.19 was released on 19 June 2026. Its notes list no atmosphere changes; the rendering items are contact shadows, parallax-corrected cubemaps, rectangular area lights behind the `area_light_luts` feature, white-furnace fixes, Solari, Vignette and LensDistortion. — [Bevy 0.19 release notes](https://bevy.org/news/bevy-0-19/)
- **Bevy 0.19.1 `bevy_light` source** (read from the crate in gearbox's cargo registry; same file at [docs.rs bevy_light 0.19.1](https://docs.rs/bevy_light/0.19.1/src/bevy_light/atmosphere.rs.html)):
  - `Atmosphere::mars(medium)`: `inner_radius` 3_389_500 m (Seidelmann 2007), `outer_radius` 3_509_500 m, `ground_albedo` `Vec3::splat(0.1)`.
  - `ScatteringMedium::mars(falloff_resolution, phase_resolution, dust_phase: Handle<Image>)`: atmosphere height 120 km.
  - *Rayleigh term:* scattering RGB (9.91e-8, 2.32e-7, 5.65e-7) m⁻¹, exponential falloff with an 8 km scale height.
  - *Dust term:* scattering RGB (30.67e-6, 25.39e-6, 20.93e-6), absorption RGB (1.26e-6, 5.25e-6, 9.33e-6), the double-exponential 4 km / 20 km curve, and `PhaseFunction::from_chromatic_texture(dust_phase)`, documented as "requires an Nx1 chromatic phase texture".
  - *Earth preset for comparison:* `ScatteringMedium::earth`, Rayleigh (5.802e-6, 13.558e-6, 33.100e-6), Mie absorption 3.996e-6, scattering 0.444e-6, asymmetry 0.8, plus an ozone tent.
  - *Source:* [mirrored atmosphere.rs](https://doc.qu1x.dev/bevy_trackball/src/bevy_light/atmosphere.rs.html)
- **Gaps in 0.19.1.** 0.19.1 has `SunDisk::EARTH` (0.00930842 rad) and `SunDisk::OFF` but **no `SunDisk::MARS`**. `lux::RAW_SUNLIGHT` = 130,000, `DIRECT_SUNLIGHT` = 100,000 and `FULL_DAYLIGHT` = 20,000. `SunDisk` changes only the disk's appearance, not the illuminance. — [docs.rs bevy_light 0.19.1 directional_light.rs](https://docs.rs/bevy_light/0.19.1/src/bevy_light/directional_light.rs.html)
- **The official example is on Bevy main.** The atmosphere example on bevy.org loads `asset_server.load("textures/mars_mie_phase.ktx2")` and creates `ScatteringMedium::mars(256, 256, mars_phase)`. Pressing key 4 switches to `Atmosphere::mars(..)`, moves the atmosphere origin to `-Y * inner_radius` and sets `sun_disk.angular_size = SunDisk::MARS.angular_size`. It keeps `illuminance: lux::RAW_SUNLIGHT` and `Exposure { ev100: 13.0 }` for both planets, and toggles `AtmosphereMode::LookupTexture` against `AtmosphereMode::Raymarched`. — [Bevy atmosphere example](https://bevy.org/examples/3d-rendering/atmosphere/)
- **Where the Mars preset came from.** It was added in Bevy PR #22884, "Add Martian atmosphere constant", with values from the Mie-theory paper. It ships a small precomputed Mie phase lookup texture, and the PR notes the texture could become a network asset. — [commit mirror](https://code.morphllm.com/bevyengine/bevy/commit/acd1091c3ee5bc66ed9ae6f2ef13eca82fcedd89)
- **Other atmosphere settings.** `AtmosphereSettings` in 0.19.1 exposes the LUT sizes (transmittance, multiscattering, sky-view, aerial-view), sample counts, `aerial_view_lut_max_distance`, `sky_max_samples` and `rendering_method: AtmosphereMode`. `AtmosphereEnvironmentMapLight` exists for image-based lighting from the sky. — [docs.rs bevy_pbr 0.19.1 atmosphere](https://docs.rs/bevy_pbr/0.19.1/src/bevy_pbr/atmosphere/mod.rs.html)
- **Volumetrics.** 0.19.1 has `VolumetricFog` (camera), `FogVolume` (box volume) and `VolumetricLight` (per light) in `bevy_light/src/volumetric.rs`. — [docs.rs bevy_light 0.19.1 volumetric.rs](https://docs.rs/bevy_light/0.19.1/src/bevy_light/volumetric.rs.html)

**How others render Mars**
- CosmoScout VR (DLR) is an open-source solar-system visualiser. Its `csp-atmospheres` plugin offers several atmosphere models, including the Mie-theory Bruneton model, with precomputed data in a submodule. The README badge says MIT, but a third-party mirror lists "other". — [cosmoscout/cosmoscout-vr](https://github.com/cosmoscout/cosmoscout-vr); [Helmholtz software entry](https://helmholtz.software/software/cosmoscout-vr)
- NASA's Eyes on the Solar System originally needed the Unity plugin (2010). It was rebuilt as a browser app in September 2022, with missions including Perseverance EDL. — [NASA's Eyes (Wikipedia)](https://en.wikipedia.org/wiki/NASA%27s_Eyes); [JPL 2022 news](https://www.jpl.nasa.gov/news/explore-the-solar-system-with-nasas-new-and-improved-3d-eyes/)
- Access Mars (JPL + Google Creative Lab, October 2017) is WebVR built from Curiosity stereo-image terrain. It was adapted from JPL's OnSight drive-planning tool. — [JPL news](https://www.jpl.nasa.gov/news/take-a-walk-on-mars-in-your-own-living-room/)
- PANGU (University of Dundee) was extended for ExoMars navigation testing with rock models, surface reflectance, atmospheric dust, a rover and dynamic shadows. It was validated against MER images. — [Mars Visual Simulation for ExoMars](https://www.academia.edu/128901558/Mars_Visual_Simulation_for_ExoMars_Navigation_Algorithm_Validation)

### Inferences
- **Effective tau of Bevy's preset.** Integrating the dust density profile gives a column of about 4,771 m. Multiplying by the preset coefficients gives vertical extinction tau ≈ 0.152 (R), 0.146 (G) and 0.144 (B), and single-scattering albedo ≈ 0.96 / 0.83 / 0.69. The SSA closely matches Albers' 0.92 / 0.80 / 0.69. Rayleigh tau is only about 0.0008 / 0.0019 / 0.0045, about 30–150× below the dust. So the stock preset is a clear-day Mars. A `dust_tau` knob maps to roughly `density_multiplier ≈ tau / 0.146`: about 3.4× for tau 0.5, about 7× for tau 1, and about 34–70× for storms of tau 5–10. Check whether `with_density_multiplier` scales the Rayleigh term too; it should not matter, because the dust dominates.
- **Recommended gearbox Mars sky** (all types exist in 0.19.1):
  1. Copy `assets/textures/mars_mie_phase.ktx2` from the Bevy repo into gearbox assets and load it as `dust_phase`.
  2. Build `ScatteringMedium::mars(256, 256, phase).with_density_multiplier(tau / 0.146)`.
  3. Spawn `Atmosphere { inner_radius: 3_389_500., outer_radius: 3_509_500., ground_albedo: Vec3::splat(0.2..0.3), medium }` on gearbox-planet's non-spinning anchor.
  4. Add `SunDisk { angular_size: 0.00611, intensity: 1.0 }` to the sun.
  5. Set `DirectionalLight.illuminance ≈ 130_000 × 0.431 × (1.524 AU / r)²` (≈ 56 klx mean).
  6. Raise exposure by about 1.2 EV against the Earth setup.
  7. Use `AtmosphereEnvironmentMapLight` so ambient light picks up the butterscotch cast.

  This replaces the current `ScatteringMedium::earth` + Rayleigh override in `crates/gearbox-planet/src/planet/mod.rs` for Mars.
- **High-tau storms.** Bruneton- and Hillaire-style LUT models are fitted to thin atmospheres; Schneegans et al. explicitly flag artefacts for dense ones. For storms (tau of 5 or more), the sky LUT alone will not give the low-contrast, sunless orange-brown murk. Pair it with `AtmosphereMode::Raymarched` and a near-ground `FogVolume`/`VolumetricFog` layer in dust colour (high albedo in red, absorbing in blue). Then cut direct sun with Beer–Lambert, e^(−tau/μ), and raise the diffuse fraction.
- **Dust devils** are best done as camera-facing or instanced volumetric columns (tens of metres wide, hundreds of metres tall, drifting at about 1 m/s), not as part of the atmosphere model. That matches how the existing bevy_weather / vegetation code adds local effects.

### Gaps
- `mars_mie_phase.ktx2` was not inspected. Its resolution, format and licence (presumably the Bevy repo's MIT/Apache-2.0) are unconfirmed, and it is not in the published `bevy_light`/`bevy_pbr` crates.
- No published Mars parameter set for Hillaire-style or Unreal SkyAtmosphere implementations was found, official or community.
- The current Eyes on the Solar System rendering engine is not named in the sources found. No public source code for Access Mars was found.
- Whether Bevy 0.20 (in development; a 0.19→0.20 migration-guide draft exists) changes the atmosphere API beyond `SunDisk::MARS` was not checked.

## 3. Greenhouse visuals: grow-light illumination, inspection lighting, transparent/inflatable greenhouse materials, dust on glazing

### Takeaway
Red+blue LED grow light looks magenta-purple, and green leaves under it look grey, black or purple because there is almost no green light to reflect. Real space-farming hardware (NASA Veggie, EDEN ISS) adds green or white so people can inspect plants, and about 5% green is enough. Glazing is typically ETFE film (about 85–95% transmittance), sometimes two-layer inflatable. On Mars it loses roughly 0.2% transmittance per sol to settled dust, which periodic cleaning resets.

### Cited Findings
- NASA Veggie on the ISS uses 132 red, 32 green and 32 blue LEDs. Gioia Massa: plants "look gray and purple" under red+blue alone, so green is added to make them look appetising to people. Ray Wheeler: green LEDs help human perception but put out less light than the reds and blues. — [KQED/NPR, "The Salad Frontier"](https://www.kqed.org/bayareabites/83133/the-salad-frontier-why-astronauts-need-to-grow-lettuce-in-space)
- Signify (Philips) says red and blue light mixes to purple, and plants in such a room look grey or black for lack of green light. Crop inspection usually works if at least 5% green is added. Daylight through glass supplies enough green. — [Philips Hortiblog](https://www.lighting.philips.com/application-areas/specialist-applications/horticulture/hortiblog/light-and-growth/myth-surrounding-white-light-part-3); [Vertical Farm Daily](https://www.verticalfarmdaily.com/article/9598805/why-you-don-t-need-full-white-light-for-crop-inspection/)
- Some grow fixtures look so purple that their vendor rates them zero stars for standalone use and expects them to be mixed with white channels. — [LEDaquaristik light comparison](https://www.ledaquaristik.de/en/Help/Light-comparison/)
- EDEN ISS (DLR Antarctic analogue at Neumayer III) runs 42 LED lamps tuned to red/blue with per-LED wavelength control. The 2018 setpoints were 330–600 µmol m⁻² s⁻¹ PPFD, 21 °C, ~65% RH, 1000 ppm CO2 and a 17 h photoperiod. It produced more than 268 kg of edible biomass in 2018, a yearly productivity of 27.4 kg/m² over 12.5 m². — [Zabel et al., Frontiers in Plant Science 2020](https://www.frontiersin.org/articles/10.3389/fpls.2020.00656/full); [Scientific American](https://www.scientificamerican.com/article/a-new-antarctic-proof-greenhouse-heads-south-to-polar-scientists)
- The University of Arizona CEAC / NASA KSC Prototype Lunar/Mars Greenhouse has four inflatable cylinders, each 2.06 m in diameter and 5.5 m long, joined by a hallway. Crops grow in a cable-suspended, substrate-less nutrient-film hydroponic system. Crew CO2 is fed in and the plants return O2. — [UA SSEL project page](https://ssel.arizona.edu/missions/prototype-lunarmars-greenhouse); [NASA article](https://www.nasa.gov/science-research/lunar-martian-greenhouses-designed-to-mimic-those-on-earth/)
- ETFE film transmittance is quoted at 90–95% (Tensaform brochure) or 85–90% (another supplier). — [Tensaform ETFE (ArchiExpo PDF)](https://pdf.archiexpo.com/pdf/tensaform/etfe/59245-377897.html); [BDIR ETFE](https://bdir.com/news/performance-and-characteristics-of-etfe-membrane)
- An inflatable Moon/Mars dome concept proposes two transparent film layers, each 0.2–0.4 mm thick, with air between them for insulation. — [Bolonkin, arXiv 0707.3990](https://arxiv.org/pdf/0707.3990)
- Real Mars greenhouse concepts have to block UV while passing enough light. In low pressure, lights and heaters may overheat because there is little convective cooling. — [Univ. Saskatchewan URJ, "Can a Greenhouse Be Established on Mars?"](https://usurj.journals.usask.ca/article/view/265); [Mars Daily](https://www.marsdaily.com/reports/Reaping_The_Fruits_Of_Their_Labor_Will_Keep_The_New_Martians_Heatlthy.html)
- Dust accumulation on Mars surfaces: about 0.2%/sol output loss (range 0.05–2%/sol), with site-dependent cleaning by gusts and dust devils. — [Lorenz et al. 2021](https://digital.inta.es/entities/publication/e629d870-297b-4f08-89ce-ee637569c063/full)
- Bevy 0.19.1 `StandardMaterial` has `specular_transmission`, `diffuse_transmission`, `thickness` and `ior`, each with optional texture inputs (`specular_transmission_texture`, `thickness_texture`, ...). `RectLight` exists in 0.19.1, and 0.19 ships rectangular area lights behind the `area_light_luts` feature. — [docs.rs bevy_pbr 0.19.1 pbr_material.rs](https://docs.rs/bevy_pbr/0.19.1/src/bevy_pbr/pbr_material.rs.html); [Bevy 0.19 release notes](https://bevy.org/news/bevy-0-19/)

### Inferences
- **Grow-light look in an RGB renderer.** Model a grow bar as `RectLight`s or spot arrays with linear colour ≈ (1.0, 0.0–0.05, 0.6–0.9): mostly red, some blue, a trace of green. A leaf albedo such as (0.05, 0.20, 0.04) then returns about (0.05, 0.0–0.01, 0.03): a very dark purple-grey. That matches the "grey, black, purple" reports without any special shader. A Veggie-like ratio (132 R : 32 G : 32 B by LED count) gives a magenta-pink cast. An "inspection mode" only needs about 5% green, or a full-white channel switched on.
- **Daylight through the dome is already orange.** It is the butterscotch skylight plus a sun of about 0.43× Earth irradiance. Greenhouse interiors will look warm and dim under natural light and purple under LEDs. A useful sim scenario is a robot camera that must switch the lights to white before running plant-health perception.
- **Dusty glazing** works as one per-pane scalar `dust` in 0–1 that grows at about 0.002 per sol and is reset by cleaning events (a robot task, or wind events). Drive three things from it:
  1. reduce `specular_transmission`;
  2. raise roughness and lerp base colour toward regolith dust (albedo about 0.25–0.3, reddish);
  3. weight it by an up-facing mask (normal·up), because dust settles on tops of panels.

  This mirrors gearbox's existing "one scalar drives ground and vegetation" approach. The same scalar can scale the PAR that reaches plants, so dirt has a growth consequence.
- **Inflatable ETFE domes** suit a thin-film material: high transmission, low IOR (about 1.4), slight haze, and Fresnel-heavy reflections at grazing angles. Two layers can be faked with one thickness value instead of two real surfaces.

### Gaps
- No measured spectra for EDEN ISS or Veggie (exact peak wavelengths and red:blue PPFD ratios) were retrieved. The common 450 nm / 660 nm peaks are not cited here.
- No ETFE transmittance data under Martian UV, spectrum or dust were found.
- No published real-time rendering technique specifically for "dust on glass" was found. The approach above is inferred from the measured deposition rates.

## 4. Game precedent (Surviving Mars, Planet Crafter, Astroneer, Space Engineers, KSP2, Mars Horizon, The Martian VR, Occupy Mars, Stardew, Farming Simulator)

### Takeaway
Every shipped game abstracts growth into a timer gated by a few binary needs (water, power, temperature, sometimes oxygen or soil). Most failure modes are "a supply was interrupted, so everything died". The best-received depth comes from soil-quality or crop-rotation trade-offs (Surviving Mars). The most common complaints are micromanagement, opaque failures and sudden total die-offs (Occupy Mars, Surviving Mars). No game found models light spectrum, PAR or plant morphology.

### Cited Findings
**Surviving Mars (Haemimont/Paradox)**
- Crops are the main food source and add a little O2 that only offsets dome demand. Crops need constant water, and a shortage makes them decay, lowering the harvest. Dry Farming halves water use.
- Hydroponic examples (growth in sols / water / harvest): Leaf Crops 4 / 0.6 / 12 food; Rice 2 / 0.9 / 8; Algae 1 / 0.6 / 4 with O2 1. Farm examples: Potatoes 4 / 2.4 / 66 with soil −10%; Soybeans 4 / 2 / 48 with soil +10%; Wheat 1.6 / 1.6 / 20 with no soil change; Cover Crops +20% soil; Corn 4 / 3 / 84 with −10% soil; Fruit Trees 6.4 sols / 106 food with +10% soil.
- Farms start at 50% soil quality, and performance runs from +50 at 100% to −50 at 0%. A disabled farm recovers 2.4% per sol up to 90%.
- Source: [Surviving Mars wiki – Food/Crops](https://survivingmars.paradoxwikis.com/Crops)
- Players report that understaffing is the top cause of failed crops, and that power or water loss mid-cycle causes failures. Crop rotation (cover crops, then soybeans, then quinoa) builds soil. — [Steam discussion](https://steamcommunity.com/app/464920/discussions/0/4841941418352682440)
- PC Gamer finds the interface demands a lot of manual juggling that clashes with the automation theme. OpenCritic's consensus notes "heavy micromanagement can be frustrating". Players describe open farms left unharvested for about 30 sols while the colony starved. — [PC Gamer review](https://pcgamer.com/surviving-mars-review); [OpenCritic](https://opencritic.com/game/5735/surviving-mars); [Steam thread](https://steamcommunity.com/app/464920/discussions/0/2949168687321915977/?ctp=2)

**The Planet Crafter (Miju Games)**
- Food Growers unlock at 12 ppt oxygen and cost 1 aluminium, 1 iron and 1 water bottle. — [Gameluster guide](https://gameluster.com/the-planet-crafter-how-to-get-more-food/)
- The T2 grower runs at a 1.1× rate and uses twice the power. The planted seed is returned at harvest but not multiplied. — [Steam discussion](https://steamcommunity.com/app/1284190/discussions/0/4333105405784921429); [TechRaptor farming guide](https://techraptor.Net/gaming/guides/planet-crafter-farming-guide); [Steam discussion on seeds](https://steamcommunity.com/app/1284190/discussions/0/3464983493950649808)
- Terraformation shows visible global stages: the sky goes from "oppressively red" to blue, then clouds, rain and liquid water appear. One player notes the new green moss "changes colour but doesn't change texture". — [TechRaptor terraforming guide](https://techraptor.Net/gaming/guides/planet-crafter-terraforming-guide); [Corrosion Hour review](https://www.corrosionhour.com/the-planet-crafter-review/); [PC Gamer preview](https://www.pcgamer.com/au/terraform-a-planet-in-this-survival-game-that-looks-like-subnautica-on-mars)

**Space Engineers (Keen), Apex Survival update (2025)**
- Algae Farms are the basic food source. They need sunlight, with efficiency set by how directly the sun hits the block, and no upkeep. Fruit, grain, mushroom and vegetable items convert to seed or spore packs, which Farm Plots grow. — [Space Engineers "Apex Survival" announcement](https://www.spaceengineersgame.com/space-engineers-apex-survival-live-now/)
- The Farm Plot uses 20 kW, stores up to 180 L of water fed by an ice-filled Irrigation System, and has a built-in light. It needs moderate temperature; Freeze or Inferno kills plants. Crops also die from lack of water or block damage, and harvesting at 100% progress gives 2–3 items. An Event Controller cannot detect ripe crops. A known bug makes the plot check only its own grid's temperature, ignoring enclosing grids. — [Space Engineers wiki – Farm Plot](https://spaceengineers.wiki.gg/wiki/Farm_Plot); [Farming](https://spaceengineers.wiki.gg/wiki/Farming)

**Occupy Mars: The Game (Pyramid Games)**
- Store copy: an open-world colonisation sandbox where you build a base, make oxygen and grow crops that must survive Mars conditions. — [Epic store page](https://store.epicgames.com/p/occupy-mars)
- The Prologue had a hydroponic shelf and incubator but no greenhouse. In the full game the composter and seed device sit in the greenhouse. — [Steam (Prologue) discussion](https://steamcommunity.com/app/1310520/discussions/0/4085282327481086005)
- Domes are farmed by adding soil through the dome control panel, as with greenhouse fields, then planting. — [Steam discussion](https://steamcommunity.com/app/758690/discussions/0/4848778928898441166)
- Players report total die-offs after building a new big dome. Causes named include brief power or water interruption, an oxygen drop and an open hangar door. The developer replied "Once you build a new dome, the temperature inside is not warm enough", heat then leaks out of the base, and a double iris airlock is "a very good workaround, but the issue's with the temperature." Plants need power, water and oxygen connected directly to the greenhouse. — [Steam discussion](https://steamcommunity.com/app/758690/discussions/0/4032473196483596877)

**Astroneer (System Era)** (no Mars setting)
- Seeds drop from dug-up flora and are planted by hand. Growth takes a couple of minutes, and harvesting can be automated with a Proximity Repeater. Tappers produce organic over time, and several tappers split one plant's output. The Megatech Biodome adds an Auto-Planter, Auto-Fertilizer and Auto-Harvester. — [Astroneer wiki – Seed](https://astroneer.wiki.gg/wiki/Seed); [Biodome](https://astroneer.wiki.gg/wiki/Biodome)

**Kerbal Space Program 2**
- Colonies, with habitation modules for surface, orbital and planetary colonies, were a promised feature. Take-Two's WARN notice covered about 70 staff at the Seattle studio with a 28 June 2024 date. The game was last updated in June 2024, and Private Division was sold in November 2024. — [Wikipedia – KSP2](https://en.wikipedia.org/wiki/Kerbal_Space_Program_2); [Neowin](https://www.neowin.net/news/kerbal-space-program-2-studio-intercept-games-is-reportedly-shutting-down/)

**Mars Horizon (Auroch Digital, ESA-supported, 17 November 2020)**
- A space-agency management game: turn-based missions, a tech tree, and a base of launch pads, labs and training facilities. Metacritic 71, OpenCritic 74. Nothing found indicates farming. — [Gematsu](https://www.gematsu.com/2020/10/mars-horizon-launches-november-17); [Steam](https://store.steampowered.com/app/765810/Mars_Horizon/); [DekuDeals](https://www.dekudeals.com/items/mars-horizon)

**The Martian VR Experience (Fox Innovation Lab / RSA / The VR Company, 2016)**
- Interactive potato segments (sorting potatoes into bins, "basketball" with potatoes) alongside plutonium recovery, solar panels in a storm, rover driving and MAV ejection. It runs about 20 minutes. Reviews were mixed: "superficial", "disjointed", and potato sorting "mundane yet still engaging". — [Press Start review](https://press-start.com.au/?p=39548); [Fortune](https://fortune.com/2016/03/13/matt-damon-in-virtual-reality); [ETCentric](https://www.etcentric.org/?p=100425)

**Stardew Valley (growth-stage reference)**
- Crops can be planted only in their season and wither when the season changes after day 28; multi-season crops continue. An unwatered day means no growth but no death. Listed grow times exclude planting day and assume watering that day. — [Stardew Valley Wiki – Crops](https://stardewvalleywiki.com/Crops)

**Farming Simulator 25 (calendar-based growth reference)**
- Growth duration is fixed in in-game months; onions take 6 months, planted March–April and harvested August–September. "Days per month" stretches real time without changing stages. Seasonal Growth on/off enforces the crop calendar, and a "Paused" growth option exists. — [FS25 wiki – Onions](https://farmingsimulator.wiki.gg/wiki/Onions/Farming_Simulator_25); [Steam discussion](https://steamcommunity.com/app/2300320/discussions/0/4634862878162261704); [DTGRE guide](https://www.dtgre.com/2024/11/farming-simulator-25-mastering-seasons.html)

### Inferences
- **Patterns gearbox can borrow:**
  - discrete growth stages with per-stage durations, as in Stardew and FS;
  - "no water means no growth, not instant death" (Stardew), a gentler and more legible rule than the Occupy Mars and Space Engineers instant kill;
  - a soil or substrate quality scalar that crops push up or down (Surviving Mars);
  - sun-dependent output (Space Engineers algae), which on Mars links naturally to tau and glazing dust.
- **Patterns to avoid**, because players found them unfair or tedious:
  - a sudden total die-off from a one-tick supply glitch or an invisible temperature drop (Occupy Mars);
  - ripe crops a robot cannot detect (the Space Engineers Event Controller limitation);
  - starvation caused by hidden job priorities (Surviving Mars).

  For a robot simulator, every plant state should be observable through sensors: leaf colour under white light, stage geometry, soil moisture.
- **Growth-rate realism.** Games compress growth to 1–6 sols (Surviving Mars) or minutes (Astroneer, Planet Crafter). gearbox should expose a time-scale knob over a physically plausible baseline, for example EDEN ISS lettuce-class crops at 330–600 µmol m⁻² s⁻¹ with a 17 h photoperiod, rather than hard-coding game-like timers.
- **The gap gearbox can fill.** No game found renders the purple grow-light look or ties growth to light quantity or spectrum.

### Gaps
- No official KSP2 statement that colonies were cancelled was found; the sources show only the studio closure and the halted updates.
- Per-seed growth times for Planet Crafter were not retrieved (the fandom wiki returned HTTP 402).
- No source was found on Space Engineers' Mars planet visuals, or on whether its farming interacts with the Mars environment.
- No dedicated review analysis of Occupy Mars farming (believable vs tedious) was found beyond Steam threads.

## 5. Robot simulator precedent (Mars and space robotics sims, agricultural sims with plant growth, Mars greenhouse robotics)

### Takeaway
Open Mars robot simulation has four strands. Gazebo/ROS worlds include the Space ROS Curiosity demo, Leo Rover Marsyards and HiRISE-derived terrain. Isaac Sim-based frameworks are Space Robotics Bench with its Mars preset, and OmniLRS, which is lunar. Separately, JPL and ESA run closed high-fidelity tools (ROAMS, ENav/HyperDrive Sim, PANGU). Agricultural sims with real plant growth are rare: Helios is the strongest open framework for growth and radiation, and Gazebo/Isaac agri work is mostly static crop scenes or harvesting. No simulator was found that combines a Mars site, a greenhouse and an agricultural robot.

### Cited Findings
**Mars and space robotics**
- **Space ROS demos** (Apache-2.0): Canadarm2, Curiosity Rover, Curiosity with Ogma runtime verification, Nav2, a Trick demo, Space Robots, Basilisk astrodynamics, plus lunar terrain and lunar pole exploration rover folders. — [space-ros/demos](https://github.com/space-ros/demos)
- **The Space ROS Curiosity demo** builds and runs in Docker (`./build.sh`, `./run.sh`) and is driven by `std_srvs/Empty` services: `/move_forward`, `/turn_left`, `/turn_right`, `/move_stop`, `/open_arm`, `/close_arm`, `/mast_open`, `/mast_close` and `/mast_rotate`. — [space-ros/demos curiosity_rover](https://github.com/space-ros/demos/tree/main/curiosity_rover)
- The original 2022 Space ROS demos showed Curiosity on simulated Mars terrain in Gazebo, with ros2_control moving the parts. — [Open Robotics community meeting, Aug 2022](https://discourse.openrobotics.org/t/community-meeting-interns-2022-aug-2022/46475)
- **The NASA Curiosity ROSject** (ROS 1 / Gazebo) has velocity and mast control, odometry, cameras and Mars terrains, using JPL's official 3D models. Users reported packaging problems, and its mass and dimensions were not updated to official figures. — [ROS Discourse](https://discourse.ros.org/t/the-nasa-curiosity-rover-rosject-is-now-open-source/7635)
- **Community rovers:** r2m_rover (ROS 1 Curiosity-style rover with a Mars world and `/cmd_vel`) and ros2_rover (a ROS 2 Humble Sawppy with `mars.launch.py`). — [r2m_rover](https://github.com/Prithvipatel007/r2m_rover); [mgonzs13/ros2_rover](https://github.com/mgonzs13/ros2_rover)
- **leo_gz_worlds** (ROS 2) ships `marsyard2020/2021/2022.sdf`, drone-scanned European Rover Challenge Marsyards from Kielce, Poland. The 2021 and 2022 files need Gazebo Garden or newer. — [ROS docs leo_gz_worlds](https://docs.ros.org/en/jazzy/p/leo_gz_worlds/); [Leo Rover blog](https://www.leorover.tech/post/exploring-mars-through-simulation)
- **A Gazebo HiRISE terrain sim** (2025) builds Martian terrain from HiRISE elevation and texture at 1 m resolution, with a rover carrying LiDAR, RGB and IMU. Terrain is rigid. — [arXiv 2509.11082](https://arxiv.org/pdf/2509.11082)
- **Space Robotics Bench** (University of Luxembourg) is built on NVIDIA Isaac Sim + Isaac Lab, dual-licensed MIT/Apache-2.0, with SimForge procedural assets under CC0.
  - Domain presets include Mars ("Martian surface with rocks", g ≈ 3.72 m/s²), Moon, asteroid, orbit and Earth. Each domain sets solar intensity, colour temperature and the sun's angular diameter.
  - Assets are generated headlessly in Blender from node blueprints, baked to PBR and exported as USD.
  - Robots include Perseverance, Ingenuity, Pragyan, Spot, ANYmal, Canadarm, Franka and UR. Atmospheric effects on robots are approximated with random forces.
  - Sources: [arXiv 2509.23328](https://arxiv.org/html/2509.23328v1); [repo](https://github.com/AndrejOrsula/space_robotics_bench); [PyPI srb](https://pypi.org/project/srb/)
- **OmniLRS** (ICRA 2024) is a photorealistic lunar simulator on Isaac Sim with procedural environments, multiple robots, ROS 1/2 bindings for robots and the environment, and a synthetic-data pipeline. YOLOv8 trained on its data comes within 5% of training on real data. It is open source. — [arXiv 2309.08997](https://arxiv.org/abs/2309.08997v1)
- **NASA Space Robotics Challenge Phase 2** ran virtual lunar ISRU competition rounds on Gazebo/ROS. No official public repo was found. — [SpaceRef](https://spaceref.com/?p=139209); [Team Mountaineers arXiv 2003.09968](https://arxiv.org/pdf/2003.09968v1.pdf)
- **JPL's closed tools.** ROAMS (Rover Modeling and Simulation) provides virtual testbeds for Mars surface-phase validation, including real-time flight-interface simulation. ENav Sim, built on the HyperDrive Sim library, was used to develop Perseverance's ENav. It generates 2.5D terrains and synthetic stereo, and models rover-terrain settling and slip. — [JPL ROAMS](https://robotics.jpl.nasa.gov/what-we-do/research-tasks/roams-rover-modeling-and-simulation); [MLNav arXiv 2203.04563](https://arxiv.org/pdf/2203.04563)

**Agricultural simulators, plant growth and plant-robot interaction**
- **Helios** (UC Davis, Bailey lab) is an open-source C++ model-coupling framework. Its plug-ins cover GPU radiation transport (direct and diffuse sun, artificial lights, longwave, camera simulation), surface energy balance, stomatal conductance, photosynthesis, solar position and procedural plant generation with dynamic growth. The 2024 extension simulates RGB, multi- and hyperspectral, thermal and depth cameras with automatic labels. PyHelios (`pip install pyhelios3d`) is incomplete. — [Bailey 2019, Frontiers](https://www.ncbi.nlm.nih.gov/pmc/articles/PMC6813926/); [2024 imaging paper](https://www.ncbi.nlm.nih.gov/pmc/articles/PMC11136674/); [PyHelios](https://github.com/PlantSimulationLab/PyHelios)
- **CropCraft** (Romea/INRAE) is a Python + Blender (4.0+) procedural crop-field generator driven by YAML. It outputs a Blender file, a Gazebo Classic model and ground-truth labels for LiDAR and cameras. — [arXiv 2511.02417](https://arxiv.org/pdf/2511.02417); [Romea/cropcraft](https://github.com/Romea/cropcraft)
- **AgROS** (Tsolakis, Bechtsis, Bochtis, Agronomy 9(7):403, 2019) is a ROS-based emulation tool. Users pick a real field from a map, add trees and objects, and test commercial UGVs. No public code was found. — [DOAJ record](https://doaj.org/article/1fc97a2ae88b40fa93135b28a0c7c4b6)
- **Gazebo Plants** (2024) is a Cosserat-rod Gazebo plugin for plant–robot interaction during harvesting, working with ROS/MoveIt. The code is available on request under MPL 2.0. fields-ignition generates random Blender tomato crops for Ignition Gazebo (work in progress). harvester-sim has a ROS Kinetic procedurally generated strawberry plant. — [arXiv 2402.02570](https://arxiv.org/abs/2402.02570v1); [fields-ignition](https://github.com/azazdeaz/fields-ignition); [harvester-sim](https://github.com/arsh09/harvester-sim)
- **Agri-Sim** (August 2026) is Unity + ROS 2/MoveIt 2 with a configurable tomato greenhouse and a dual-arm mobile harvesting robot (RGB-D, LiDAR, IMU). Its code release is not stated. — [arXiv 2608.29100](https://arxiv.org/abs/2608.29100)
- **Isaac Sim agriculture.** Agri-FoodTEF offers a photorealistic virtual greenhouse on Isaac Sim with ROS 2, configurable in size, lighting and row spacing. Pheno-Robot builds orchard harvesting environments in Isaac Sim with fragile-joint fruit. A greenhouse simulation environment for an indoor vegetable greenhouse also exists. — [Agri-FoodTEF](https://agrifoodtef.eu/catalogue-of-services/provision-virtual-playground-robots); [Pheno-Robot arXiv 2402.09685](https://arxiv.org/pdf/2402.09685); [arXiv 2107.14348](https://ar5iv.labs.arxiv.org/html/2107.14348)

**Real Mars and space greenhouse analogues (ground truth for a sim)**
- **EDEN ISS:** PPFD, temperature, RH, CO2, photoperiod and yield figures as in section 3. — [Frontiers 2020](https://www.frontiersin.org/articles/10.3389/fpls.2020.00656/full)
- **UA Prototype Lunar/Mars Greenhouse:** inflatable NFT hydroponic modules (section 3). — [UA SSEL](https://ssel.arizona.edu/missions/prototype-lunarmars-greenhouse)

### Inferences
- The most reusable open pieces for gearbox are:
  - leo_gz_worlds Marsyard scans, as a known benchmark terrain;
  - HiRISE DTMs, for real sites;
  - Space Robotics Bench's domain-preset idea (gravity, sun intensity, colour temperature, angular size per body), which maps directly onto gearbox env knobs;
  - Helios's growth and radiation model, as a reference for driving plant growth from absorbed PAR.
- A Rust/Bevy sim cannot link Helios's GPU (OptiX/CUDA) plug-ins easily. Port the concepts instead: per-plant light interception, then photosynthesis, then biomass and stage geometry, evaluated at low rate.
- The empty niche: no open simulator joins Mars lighting and atmosphere, a pressurised greenhouse and an agricultural manipulator. Space Robotics Bench has Mars and robots but no plants. Agri sims have plants but Earth daylight. EDEN ISS and Veggie provide real parameters to anchor such a sim.

### Gaps
- No project literally named "MarsSim" was found. It may be an alternate or unpublished name.
- No public NVIDIA "Isaac Sim Mars" environment from NVIDIA itself was found.
- The Space ROS Curiosity README does not state its ROS 2 distro, Gazebo version, or terrain and mesh provenance.
- "AgriSim" as a distinct older project was not found; only Agri-Sim (2026) turned up.
- No open simulator of a robot inside a Mars or space greenhouse was found.

## 6. Freely usable Mars assets (NASA 3D, HiRISE, USGS, CC0 textures) and licences

### Takeaway
NASA and USGS Mars data (HiRISE DTMs and orthoimages, Viking MDIM mosaics, NASA 3D models) is effectively public domain in the US. Credit is requested, and NASA logos and endorsement are restricted. The AWS copy of the HiRISE DTMs is explicitly CC0 and comes as Cloud-Optimised GeoTIFF. Poly Haven and ambientCG textures are CC0. Space Robotics Bench's procedural assets are CC0.

### Cited Findings
- **NASA-3D-Resources** (GitHub, 3D models, 3D-print files, images and textures): "free and without copyright", subject to NASA's media guidelines. — [nasa/NASA-3D-Resources](https://github.com/nasa/NASA-3D-Resources)
- **NASA media guidelines:** content used to render 3D models (textures, polygon data) is generally not subject to US copyright. The insignia and logotype are not public domain. Commercial use must not imply endorsement. Identifiable people and third-party-copyrighted images need permission. — [NASA images and media guidelines](https://nasa.gov/nasa-brand-center/images-and-media)
- **HiRISE images:** public domain with no usage limits; credit NASA/JPL-Caltech/UArizona where possible. — [HiRISE image page](https://hirise.lpl.arizona.edu/ESP_021954_1230)
- **HiRISE DTMs on PDS:** the MRO HiRISE DTM V1.0 (DTMs plus orthoimages) is listed as US public domain. — [data.gov](https://catalog.data.gov/dataset/mro-mars-high-resolution-imaging-science-experiment-dtm-v1-0-145f7)
- **HiRISE DTMs on AWS:** the NASA/USGS collection is CC0 1.0, tied to MOLA as a geodetic frame, and served as Cloud-Optimised GeoTIFF. — [AWS Registry of Open Data](https://registry.opendata.aws/nasa-usgs-mars-hirise-dtms/)
- **HiRISE DTM listing:** 1,338 entries as of the fetch. — [uahirise.org/dtm](https://www.uahirise.org/dtm/)
- A 1 m HiRISE-derived terrain has already been used in Gazebo. — [arXiv 2509.11082](https://arxiv.org/pdf/2509.11082)
- **Viking MDIM 2.1 mosaics:** greyscale at 256 px/deg (~231 m/px) and colorized at ~232 m/px. Ames warped the colour over the B/W MDIM 2.1. Positional accuracy is about 200 m (one pixel), against 3 km for MDIM 2.0. — [USGS Astrogeology MDIM2.1 grayscale](https://astrogeology.usgs.gov/search/map/viking_mdim2_1_grayscale_global_mosaic_232m); [Colorized](https://astrogeology.usgs.gov/search/map/mars_viking_colorized_global_mosaic_232m); [USGS paper](https://pubs.usgs.gov/publication/70201627)
- **Poly Haven:** all assets CC0 and usable commercially. You may not claim authorship or relicense, and a link back is requested. — [Poly Haven FAQ](https://polyhaven.com/faq)
- **Poly Haven Namaqualand scans** (October 2024): more than 30 desert plant, rock and ground-debris scans, including 5 rock and ground materials, as .blend, FBX, glTF and USD with up to 8K textures. They are a usable stand-in for arid ground. — [CG Channel](https://www.cgchannel.com/2024/10/download-poly-havens-free-namaqualand-3d-scan-library/)
- **ambientCG:** a CC0 PBR material library up to 8K, per secondary summaries. — [licenseorg ambientCG guide](https://licenseorg.com/guide/3d-assets/ambientcg)
- **Space Robotics Bench:** software MIT/Apache-2.0, SimForge assets CC0 1.0, including a Perseverance model. — [lib.rs srb_py](https://lib.rs/crates/srb_py); [arXiv 2509.23328](https://arxiv.org/html/2509.23328v1)
- **Paid "Mars" packs** (CGHawk Martian Surface 8K, CGTrader Mars packs) are not CC0 by default. — [RenderHub](https://www.renderhub.com/cghawk/martian-surface-8k-pbr-texture); [CGTrader Mars terrain pack](https://www.cgtrader.com/3d-model-packs/mars-terrain-asset-pack)

### Inferences
- **A practical asset stack for a gearbox Mars greenhouse site:**
  1. Terrain heights from an AWS CC0 HiRISE DTM COG at about 1 m/post, at a real site such as Jezero or Gale. Use MOLA/HRSC for far-field context.
  2. An albedo or detail tint from the HiRISE orthoimage, colour-shifted toward measured dust albedo of about 0.25.
  3. Close-up ground from CC0 Poly Haven or ambientCG rock and sand, tinted reddish.
  4. Rover and robot meshes from NASA-3D-Resources (no NASA logos on derived branding) or SRB CC0 assets.
  5. The Bevy `mars_mie_phase.ktx2` texture for the sky.
- gearbox's memory notes that .blend files are the source and USD is the exchange format. That matches SRB's Blender-to-USD procedural pipeline, which could be used as a reference workflow.

### Gaps
- A MOLA/HRSC blended 200 m DEM was not confirmed (download location and licence unverified in this session).
- No ambientCG licence page was fetched; that relies on third-party summaries.
- No CC0 Poly Haven asset that is explicitly Mars-like (red regolith) was confirmed by name.
- The Murray Lab CTX global mosaic and Mars Trek were not checked for licence.
- The licence of Bevy's `mars_mie_phase.ktx2` was not verified.
