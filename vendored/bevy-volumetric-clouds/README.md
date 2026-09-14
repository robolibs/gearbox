# Bevy volumetric clouds — local runtime

Runtime and shaders from [evroon/bevy-volumetric-clouds](https://github.com/evroon/bevy-volumetric-clouds),
adapted to Gearbox's Bevy 0.19 embedded viewport. See [UPSTREAM.md](UPSTREAM.md) for the source
commit and local changes, and [LICENSE](LICENSE) for the original MIT copyright.

This is the actual upstream cloud-density, noise-generation, volumetric raymarching,
self-shadowing, and scattering implementation, with local rendering integration fixes.

## Use

Add `CloudsPlugin`, and mark exactly one perspective scene camera with `CloudsCamera`.
The application supplies its own directional light and camera controls.
`CloudsConfig` controls coverage, layer altitude, density, ray/shadow sample counts,
sun direction/radiance, zenith/horizon radiance, wind, temporal accumulation, and render scale.

Cloud output follows the selected camera viewport at `render_scale` (default 0.5),
capped at 1280 pixels wide. The noise atlas is 512×512; Worley noise is 32³.
RGBA16F images work without optional 32-bit float filtering. Previous-frame cloud colour
is copied into a separate history texture before each cloud pass. Moving the camera or
changing its projection or sunlight resets temporal accumulation.

The sky pass stays behind scene geometry and does not cast mesh shadows.
Camera controls, debug UI, and examples from upstream are intentionally excluded.

## Limitations

- One cloud-rendering camera per app.
- The camera should remain below the cloud layer; this is not a depth-aware cloud volume.
- Clouds shadow themselves, not terrain.
- The sky uses configurable colour gradients and a solar disk, not physical atmosphere LUTs.

## Upstream research credits

- Andrew Schneider and Nathan Vos, [The real-time volumetric cloudscapes of Horizon Zero Dawn](https://www.guerrilla-games.com/read/the-real-time-volumetric-cloudscapes-of-horizon-zero-dawn).
- Sébastien Hillaire, Physically Based Sky, Atmosphere and Cloud Rendering in Frostbite.
