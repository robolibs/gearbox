# Environment

`EnvironmentSettings` owns the scene-wide sun, indirect illumination, exposure,
bloom, fog, and cloud configuration. `skybox/` integrates the actual runtime from
[evroon/bevy-volumetric-clouds](https://github.com/evroon/bevy-volumetric-clouds),
copied into `vendored/bevy-volumetric-clouds/` with its MIT license and pinned provenance.

The old procedural cloud sphere is removed. The vendored renderer generates Perlin/Worley
noise on the GPU, raymarches a volumetric cloud layer, evaluates self-shadowing and dual-lobe
scattering, and composites the result behind scene geometry.

## Defaults

- The default sun elevation preserves the accepted daylight: indirect intensity 3000,
  exposure EV100 13.5, ambient fill, fog and bloom.
- Clouds, the visible solar disk, shadows, fog, and indirect fill share the scene sun.
- Cloud layer: 3800–5300 m, above the camera's 3000 m altitude limit.
- 96 midpoint view-ray samples, 6 shadow samples, 55% base coverage with regional weather variation.
- Half-viewport cloud rendering, capped at 1280 pixels wide; resizes with the viewport.
- Cloud output explicitly uses linear filtering, independent of the host's nearest sampler.
- Sub-sample cloud erosion fades toward its mean as ray steps/pixels grow larger.
- Separate previous-frame history, discarded when the camera, projection, or sunlight changes.
- Independent Worley feature coordinates and resolution-bounded noise octaves.
- Warped, rotated multi-scale cloud shapes, regional coverage and variable cloud thickness.
- Wind advection at (12, 0, 4) m/s on the real-time clock, plus slow internal detail evolution.
  Cloud motion is independent of physics pause; temporal accumulation has a fixed real-time scale.
- No extra sun, camera controls, or debug UI from the upstream examples.

`EnvironmentSettings.clouds` updates the active cloud renderer at runtime; `render_resolution`
is maintained by the selected viewport and `render_scale` controls its scale. Resizing replaces
the output, sky and history images together and refreshes the sky material's texture bindings.
History is reused only after a successful matching camera/configuration/texture frame.

Terrain geometry, vegetation, field bounds, and wheel tracks remain independent of the sky.
A mixed-field scene shares one environment.

## Sun

`EnvironmentSettings.sun_position` is a direction toward the sun, not a local point light.
Changing this resource at runtime updates the directional light, cloud lighting, sky palette,
fog, ambient fill, and hemispherical environment map together. There is no automatic day cycle.
The solar disk is approximately 0.53 degrees across, with HDR radiance and cloud occlusion.

`daylight.rs` approximates wavelength-dependent atmospheric attenuation using relative air
mass: lower sun angles reduce illuminance and shift the direct light toward amber. Indirect
light transitions through twilight without a sudden horizon switch; direct light switches
off below the horizon. This is an artistic RGB atmosphere approximation, not spectral
atmosphere integration or cloud-derived global illumination. Exposure remains fixed.
The air-mass approximation follows Kasten–Young, also used by
[Sandia PV_LIB](https://github.com/sandialabs/MATLAB_PV_LIB/blob/master/pvl_relativeairmass.m).
RGB extinction coefficients and twilight palettes are artist-tuned.
Sky fill stays at its daylight intensity down to four degrees of sun elevation, then
fades through twilight independently of direct-beam attenuation. Its low-sun palette
retains cool skylight rather than making every shadow dark orange.

Launch overrides accept degrees; azimuth zero points toward +Z, 90 toward +X:

```sh
GEARBOX_SUN_ELEVATION=8 GEARBOX_SUN_AZIMUTH=140 make run BACKEND=wayland
```

Without overrides the solar calendar starts at 15:00 solar time on 21 June, latitude 52° N.
Elevation overrides are clamped to -90..90;
azimuth wraps at 360. Invalid/non-finite values use the defaults.

## Environment panel

The sun icon in the left ribbon opens **Environment**, also available in the command palette.
`GEARBOX_OPEN_PANEL=environment` opens it at startup. Controls update live:

- Cloud cover: 0–100%, with Clear, Broken clouds and Overcast presets.
- Time of day: 0–24 **solar hours**, not civil time or a timezone; noon is the highest sun.
- Day of year: 1–365, with a month/day readout (non-leap calendar).
- Latitude: −90° to +90°; defaults to 52° N.

The calendar uses the approximate solar declination and hour-angle equations in
[NOAA's solar position reference](https://gml.noaa.gov/grad/solcalc/solareqns.PDF).
World +Z is north and +X is east. Date changes sun elevation, direction and daylight duration,
not field crops or their materials. Sun direction drives sky, cloud lighting, fog and indirect
fill together. High overcast also reduces scene direct sunlight with a global artistic factor;
it is not spatial cloud-shadow sampling. The View light multiplier is applied after daylight.

Launch sun overrides remain active until a calendar slider changes or **Use calendar** is
pressed; cloud adjustments alone preserve them. Cloud drift runs on real time even with
physics paused. Calendar time stays at the chosen hour. Controls are session-only.

## Limits

This is a ground-view cloud sky, not depth-aware flight through clouds. Cloud self-shadowing
does not cast moving cloud shadows onto terrain, and the sun-tinted hemispherical indirect-light
map is not regenerated from the clouds. These are upstream/integration limits, not extra
atmospheric features claimed by this integration.
