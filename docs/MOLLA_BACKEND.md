# Molla backend integration

`GEARBOX_PHYSICS=molla` selects the CPU Featherstone backend. The default
remains Rapier. Both implementations share Gearbox's existing body, collider,
joint and world traits; the Molla adapter is in `bin/gearbox/src/physics/molla`.

## Current implementation

- Stable handles, runtime body/collider/joint edits, re-rooting, mass/inertia
  queries, forces, impulses, axis limits/locks and motor controls use Molla's
  runtime scene directly. Accessors share a mutex-protected world rather than
  maintaining stale physics copies.
- Collider insertion/removal and mass-affecting changes recompute parent mass.
  Body additional mass is separate from collider contributions. Collider world
  poses and ray queries read current body transforms without a simulation step.
- Connected-body filtering, per-step pair exclusions, collision masks, material
  combine rules and per-contact impulse readout map through the existing trait.
- Internal state quarantine is exposed through backend body IDs and forwarded
  to `PhysicsWorld`'s entity report after each step.
- All Molla joints are reduced-coordinate joints, including requests that would
  select Rapier constraint joints. `joint_is_reduced` reports this truthfully.
- Soft joints lower to D6 spring freedoms while retaining their logical kind,
  authored frames and semantic motor axes. Runtime frame edits preserve live
  body kinematics and change the spring rest frame. Hitch capture's 8-to-30 Hz
  updates drive real Featherstone forces rather than retaining rigid locks.
- `internal_iterations` maps to twice that many Featherstone substeps, bounded
  to 2–128. Featherstone's direct articulated solve does not use Rapier's outer
  iteration count. Requested settings remain readable.
- Scene wheel metadata registers stable wheel and spin-joint handles with the
  articulated tyre-force layer. Rolling direction follows the bearing/knuckle,
  not the spinning wheel; opposing authored axles receive opposing signed motor
  targets for the same forward command. Geometry/mass changes refresh registration.
- Meadow heightfields, USD terrain meshes and the flat ground slab register as
  tyre ground. Wheel-ground shape pairs are masked from penalty contact without
  masking obstacles. Terrain friction grids are accepted through the backend API;
  current scene builders supply their existing scalar ground/material friction.
- Traction budgets consume mean tyre normal/friction loads. Wheel stamping uses
  tyre contact points and contact state, while link values expose tyre slip,
  slip angle and normal force. Rapier keeps its contact-based path.

## Not yet accepted

Sleeping/waking are no-ops and bodies report never sleeping. CCD is a stored
no-op, as allowed by the Molla integration
specification. Live machine acceptance remains unfinished. Do not treat this
checkpoint as a production backend release.

Tyre parameters currently use nominal supported mass per wheel: 4% radial
deflection at nominal load, damping ratio 0.7, and load-scaled smooth slip gains.
These are initial parameters, not validated tractor calibration. Current field
profiles expose visual ground materials, not a physics friction grid; connecting
authored per-field/cell friction remains separate from the tested grid API.
Deformable terrain, MPM coupling and the live drive/track/slope gates remain.

Compliance uses scalar backward-Euler force regularization with effective
articulation inertia, not a fully coupled implicit spring solve. Static
deflection depends on the substep size. Soft angular coordinates use D6's XYZ
chart; unwrapped multi-turn soft revolute position targets are not implemented.
The hard revolute path retains its scalar coordinates.

Closed joint loops remain unsupported. Invalid body descriptions or impossible
joint insertions fail explicitly; fallible runtime edits log rejection and
retain the last valid operation state. Collider replacement followed by mass
recomputation is not yet one combined transaction and needs failure-path
hardening before final acceptance.

## Build notes

Use the repository Nix environment; the host Rust compiler is too old for this
Gearbox checkout. `nix develop --impure -c oslo make check` reaches the binary
but currently fails in the unchanged `tests/oxbo_transforms.rs` integration test,
which references removed `openusd::Stage`/`usd_schema` APIs. The specification's
binary test suite is separately runnable with:

```sh
nix develop --impure -c cargo test -p gearbox-sim --bin gearbox
GEARBOX_PHYSICS=molla nix develop --impure -c cargo test -p gearbox-sim --bin gearbox
```

The original 43 binary tests plus fourteen new backend/integration checks pass
with explicit `GEARBOX_PHYSICS=rapier` and `GEARBOX_PHYSICS=molla` (57 tests each). The new
checks include real capped motor motion, mass/impulse response, stable handles,
heightfield rays/live bounds, pair filtering/contact impulses, quarantine and
its entity-report propagation, compliant frame motion and the actual
`PhysicsWorld::capture_hitch` / fixed-step capture loop.
Tyre checks cover grid friction, load readout without penalty manifolds,
opposite axle signs, configuration failure/removal, and a headless ECS scene
exercising registration, traction and elevated contact-point stamping. The ECS
tyre assertions run with Molla selected; Rapier exercises its unchanged fallback.
This does not establish any of the live tractor/hitch/PTO/slope acceptance gates.

Strict Clippy is not green in this checkout: dependency-inclusive linting finds
existing issues in `gearbox-api`, `gearbox-fields` and vendored clouds; the
`--no-deps` binary lane finds seven existing issues outside the Molla adapter.
No lint suppressions or unrelated source fixes were added to hide those failures.

Local path dependencies currently expect Molla at `../../OUSD/molla` relative
to the Gearbox worktree root. No dependency versions were refreshed. Rapier's
`enhanced-determinism` feature must match the shared Parry feature enabled by
Molla; otherwise Rapier selects incompatible hash-set drain calls.
