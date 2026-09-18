# Upstream source

- Project: https://github.com/evroon/bevy-volumetric-clouds
- Commit: ad4422ff95c0b73f9d4477a5ab84fc7634262ce0
- Retrieved: 2026-09-14
- License: MIT, original copyright retained in LICENSE.

The runtime and WGSL shaders are copied from upstream. Examples, optional camera controls,
debug UI, development features, and release tooling are not included.

Local integration changes:

- Viewport resize replaces the three screen textures and refreshes material bindings together.
- Temporal history keys the last completed camera/configuration/texture frame; incomplete GPU
  image sets wait before dispatch, and weather changes invalidate history.
- Coverage has an explicit clear-sky endpoint and tapers weather-map variation at full overcast.

- Bevy 0.19 workspace dependency and explicit cloud-camera selection.
- The application owns sunlight and camera controls; the library does not spawn another sun.
- Filterable RGBA16F cloud images, bounded dispatch, and independent noise-atlas dimensions.
- Separate previous-frame cloud history; camera, projection, and sunlight changes invalidate accumulation.
- Viewport-sized cloud output and a PBR-compatible sky material without sprite imports.
- Sky geometry follows the camera without casting shadows or clipping at the far plane.
- Bilinear atlas and trilinear Worley sampling, periodic negative coordinates, and corrected
  camera-altitude sphere intersections. Cloud haze is tuned for the higher scene cloud layer.
- Configurable zenith/horizon radiance and a cloud-occluded, antialiased 0.53-degree solar disk.
  Gearbox drives the shared sun and sky palette from its elevation-dependent daylight model.
- Linear output sampling, deterministic midpoint rays, and footprint-filtered detail erosion.
  Removed the upstream world-X density multiplier.
- Independent XYZ Worley feature offsets, periodic cell wrapping and noise octaves bounded
  by the atlas/volume resolution.
- A broad weather channel, domain warping and rotated multi-scale base sampling vary cloud
  coverage and thickness without repeating the same formations across the sky.
- Real-time wind advection in density coordinates, slow detail evolution and frame-rate-independent
  temporal accumulation. Wind no longer translates the planet-intersection ray origin.

The upstream Perlin/Worley noise, density erosion, volumetric raymarching, self-shadowing,
and dual-lobe scattering remain the basis of this renderer.
