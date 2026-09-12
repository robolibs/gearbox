# Open work

What the finished plans (`PLAN.md`, `PLAN_COM.md`, `PLAN_CLI.md`,
`PLAN_TOOLS.md`, removed 2026-09-12) left undone. The specs in `specs/` and
`CLI.md` describe what exists; `PLAN_WHEELS.md` is the one plan still open,
and it must be rewritten against `bin/gearbox/src/controller.rs` before it
can start, since it targets the stale `gearbox-core` / `gearbox-physics`
crates.

## Runtime

- Physically simulated wheels instead of the raycast vehicle
  (`PLAN_WHEELS.md`, retargeted).
- `run --headless`: the sim cannot start without a window.
- Generated OpenUSD API schemas for `GearboxMachineAPI`,
  `GearboxControllerAPI`, `GearboxLinkAPI`, `GearboxCouplingAPI`,
  `GearboxElementAPI` (`SCHEMA.md` documents them; discovery reads raw
  attribute names).

## CLI

- `gearbox shell`: a REPL keeping one agent open.
- Runtime allowlist changes, once agentio can add a peer to a live agent.

## Upstream asks against agentio

- Python bindings for `Agent` (identity, `FrontDoor` resolution, typed
  `datapod_*` clients).
- Host-local rendezvous: enumerate agents on this host without their ids.
- Runtime `allow_peer` on a live agent.
- `publish_in` and a per-agent participant prefix.
- `query_directory` continues past a bad candidate.
- Feed the directory from `peerbus::Node::hosted_topics()`.
