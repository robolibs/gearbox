# Asset notes

Dated notes about specific external machine assets and manual runs. They are
not part of any contract; the contracts are in `specs/`.

## Oxbo harvester drivetrain (2026-09-14)

Moved from `specs/DRIVETRAIN.md`.

`~/data/code/OUSD/machines/usd/oxbo_harvester.usdz` authors all six roll
joints as powered, no passive wheels, a 35,000 N·m per-wheel ceiling and a
291 kW drive-power budget. This is a custom simulation upgrade.
[Oxbo's EPD540e brochure](https://www.oxbo.com/app/uploads/2022/08/EPD540e_ENG.pdf)
lists four-wheel drive and 291 kW engine output, not a factory six-wheel-drive
specification or 35,000 N·m wheel torque. The drive budget does not subtract
auxiliary power losses.

The original package is preserved at
`~/data/code/OUSD/machines/.backups/20260914-6wd/oxbo_harvester.usdz`.
Geometry and articulation are unchanged. The Blender source is unchanged;
exporting it again requires reapplying this drivetrain metadata. The package
is self-contained and repacked with OpenUSD's `usdzip`, not a generic ZIP
writer. No real-machine safe slope or towing rating is claimed.

### Rear-wheel observation

During manual driving on the mixed-field terrain, one rear wheel had zero
solved grip while its shaft tracked about 2.58 rad/s against a 2.59 rad/s
target. A parked unloaded rear wheel measured about 0.00005 rad/s. The drive
panel reported five loaded wheels rather than counting its braking torque as
contact. The asset has vertical suspension joints on the middle wheels but
rigid rear steering mounts, so rear contact can still be lost on uneven
terrain. Axle articulation or suspension authoring is a remaining asset task,
not something shaft control solves.

## Oxbo harvester steering (2026-09-14)

Moved from `specs/STEERING.md`.

`/home/bresilla/data/code/OUSD/machines/usd/oxbo_harvester.usdz` has front,
middle and rear axle Y positions −2.5, −0.9 and 2.36 m. The fixed middle axle
sets the pivot line. Front and rear lever arms are 1.6 and −3.26 m, so fixed
degree offsets and constant angle multipliers do not give exact rolling
geometry; those overrides were removed from the asset. Joint limits are ±16°
front and ±29° rear; the controller ceiling is 29°. Six powered wheels and the
35,000 N·m per wheel, 291 kW drive budget are kept.

The pre-steering package is backed up in
`/home/bresilla/data/code/OUSD/machines/.backups/20260914-ackermann/`. The
Blender source was not changed; re-exporting it requires preserving this
metadata.

### Manual runs

- 1.5 m/s, 0.05 rad/s turn: about 1.43 m/s and 0.04–0.05 rad/s. Front targets
  3.18°/2.93°, rear −6.46°/−5.97°. With the earlier servo, steering errors
  reached about 5–7° under load; the torque-scaled servo brought sampled
  settled errors below 0.8°. A slow-turn observation, not proof of zero slip
  on every slope or trailer load.
- With the per-frame load-aware fallback servo, 1.5 m/s, 0.15 rad/s turn:
  about 1.49–1.52 m/s and 0.15 rad/s. One settled sample had front
  targets/actuals 10.32°/10.29° and 8.12°/7.99°, rear −20.33°/−20.57° and
  −16.23°/−16.23°. The unloaded outside rear wheel spun at 2.70 rad/s against
  a 2.73 rad/s target. Logs were written to `/tmp/gearbox-steering-ready.log`
  and `/tmp/gearbox-steering-final-turn.log`, screenshot
  `/tmp/gearbox-steering-final.png`; they are not kept in the repository.
