# Changelog

## [0.0.5] - 2026-07-03

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Refactor project metadata and remove macos-x86_64

## [0.0.4] - 2026-07-03

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Refactor project metadata extraction

## [0.0.3] - 2026-07-03

### <!-- 0 -->⛰️  Features

- Add GitHub Actions for release automation
- Update dependency references to git

## [0.0.2] - 2026-06-06

### <!-- 0 -->⛰️  Features

- Remove unneeded gearbox-core, -physics, -viz crates
- Implement session-based command velocity control
- Improve visual wheel spin for Ackermann steering
- Implement Ackermann steering differential
- Add per-axle steering multiplier
- Add Oxbo pea harvester capabilities
- Integrate USD-based terrain for enhanced worlds
- Robot-usd
- Add USD pose feedback and marker API
- Add local terrain mesh with physics and raycast vehicle
- Implement fully physical wheel simulation
- Rework simulator to use new USD pipeline
- USDs section in workspace tree
- Inspector shows USD world pose + geo
- Selection ring extends to USD entities
- Teleport USD bodies to visuals while paused
- Load USD ribbon + USD selection + gizmo on USD roots
- USD support via --usd, mirrors play/pause
- Bevy_frost ribbon + transport panels
- Grid layout + AnimPlugin wiring
- Gearbox-world plugin + multi-USD demo
- Basis-aware reconciliation + placeholder colliders
- Add probe-usd-sim + franka-demo bins
- Add gearbox-usd crate
- Add scene reset API and multi-vehicle script
- Implement vehicle spawn API with Zenoh
- Add USD asset integration for vehicles and markers
- Add pluggable Zenoh APIs for vehicle control
- Integrate transform-gizmo-bevy from bevy_glacial
- Extract generic UI kit into bevy_frost crate
- Add Zenoh-based tool API and WebTransport link
- Remove gamepad support from gearbox-viz
- Refactor heading indicator to a single chevron
- Add heading arrows for selected vehicles
- Add adjustable UI glass opacity
- Upgrade Rapier to f64 precision
- Implement frosted glass UI theme
- Improve gizmo visuals and picking
- Implement 2D overlay gizmo for editor
- Restructure into a cargo workspace
- Add gamepad input and new power/container systems
- Add AGROINTELLI Robotti and omni drive mode
- Add `Drone` drive mode with arcade flight
- Improve camera fly animation for cinematic framing
- Implement cinematic camera fly-to animation
- Add responsive local ground grid
- Revamp gizmos as Bevy meshes, add inspector
- Dynamic accent colors in editor
- Enhance vehicle wheel rendering with tread textures
- Improve vehicle physics, editor workflow, and UI
- Implement drag-to-place vehicle spawning
- Add skybox, clouds, and atmospheric fog to scene
- Implement multi-part vehicle presets
- Add editor UI settings panel
- Refine ground grid visual appearance
- CPP-PARITY
- Optimize grid mesh rebuilding
- Refine editor UI and grid rendering
- Implement hierarchical grid LOD fading
- Init

### <!-- 2 -->🚜 Refactor

- Use usd_bevy::physics::RapierAdapterPlugin

