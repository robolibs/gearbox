# Open work

What the finished plans (`PLAN.md`, `PLAN_COM.md`, `PLAN_CLI.md`,
`PLAN_TOOLS.md`, removed 2026-09-12) left undone. The specs in `specs/` and
`CLI.md` describe what exists; `PLAN_WHEELS.md` is the one plan still open.

## Runtime

- The rest of `PLAN_WHEELS.md`: the differential drive on tyre contact,
  tyre materials in the assets.
- `run --headless`: the sim cannot start without a window.
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
- Runtime allowlist changes over the bus (`Agent::allow_peer` exists upstream
  since agentio ea60574; the host still only reads its allowlist at launch).

