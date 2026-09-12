# Open work

What the finished plans (`PLAN.md`, `PLAN_COM.md`, `PLAN_CLI.md`,
`PLAN_TOOLS.md`, removed 2026-09-12) left undone. The specs in `specs/` and
`CLI.md` describe what exists; `PLAN_WHEELS.md` is the one plan still open.

## Runtime

- Physically simulated wheels instead of the raycast vehicle
  (`PLAN_WHEELS.md`).
- `run --headless`: the sim cannot start without a window.
- Transform gizmo in the viewport: dropped with the mara re-host
  (transform-gizmo-bevy needs a Bevy window); the Selection pane edits a
  paused asset's pose with drag values. A picking-based gizmo would have to
  be drawn with Bevy gizmos against `BevyViewportInput`.
- Outliner context menu (fly / fit / copy path / expand all) has no mara
  tree-body equivalent yet; click flies, double-click fits.
- Physics markers are attached once per root (`PhysicsAttached`); a
  variant switch reprojects the stage without re-attaching them.
- Generated OpenUSD API schemas for `GearboxMachineAPI`,
  `GearboxControllerAPI`, `GearboxLinkAPI`, `GearboxCouplingAPI`,
  `GearboxElementAPI` (`SCHEMA.md` documents them; discovery reads raw
  attribute names).

## CLI

- `gearbox shell`: a REPL keeping one agent open.
- Runtime allowlist changes, once agentio can add a peer to a live agent.

