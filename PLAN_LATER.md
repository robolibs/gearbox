# Open work

What the finished plans (`PLAN.md`, `PLAN_COM.md`, `PLAN_CLI.md`,
`PLAN_TOOLS.md`, removed 2026-09-12) left undone. The specs in `specs/` and
`CLI.md` describe what exists; `PLAN_WHEELS.md` is the one plan still open.

## Runtime

- Physically simulated wheels instead of the raycast vehicle
  (`PLAN_WHEELS.md`).
- `run --headless`: the sim cannot start without a window.
- Generated OpenUSD API schemas for `GearboxMachineAPI`,
  `GearboxControllerAPI`, `GearboxLinkAPI`, `GearboxCouplingAPI`,
  `GearboxElementAPI` (`SCHEMA.md` documents them; discovery reads raw
  attribute names).

## CLI

- `gearbox shell`: a REPL keeping one agent open.
- Runtime allowlist changes, once agentio can add a peer to a live agent.

## Upstream asks against agentio

- Requests (`/gearbox/usd/load`, `/gearbox/usd/delete`) sent to a host
  that has since restarted arrive again at the new host within a second of
  it binding. The host now drops requests whose `sent_at` prop predates its
  start; the transport should not redeliver answered requests at all.
- Python bindings for `Agent` (identity, `FrontDoor` resolution, typed
  `datapod_*` clients).
- Host-local rendezvous: enumerate agents on this host without their ids.
- Runtime `allow_peer` on a live agent.
- `publish_in` and a per-agent participant prefix.
- `query_directory` continues past a bad candidate.
- Feed the directory from `peerbus::Node::hosted_topics()`.
