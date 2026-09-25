# Contact-driven machines and towing

Ackermann machines use joint motors with three independent limits:

1. Tyre grip: 90% of the solved contact normal load × combined friction × wheel radius.
2. `gearbox:controller:drive:maxWheelTorqueNm`: maximum torque at each driven wheel.
3. `gearbox:controller:drive:maxPowerKw`: total mechanical drive power shared by powered wheels.

Unloaded powered wheels receive a small shaft-acceleration budget (`mass × radius² × 4 rad/s²`),
still bounded by authored torque and shared power, and track the differential speed target.
Slip clamping applies only with solved support. This rotates the wheel through its physical
joint without inventing a ground contact or chassis force. Loaded wheels retain their grip limit.
Parking brakes act on the wheel shaft independently of ground load, using the authored wheel
torque ceiling (6,000 Nm fallback). A lifted wheel therefore stops rather than free-spinning
indefinitely. The contact solver, not brake torque alone, determines available holding traction.
Loaded-wheel telemetry counts solved support, not nonzero motor torque.

Power reduces the torque envelope at speed. Low-speed pulling is bounded primarily by wheel
torque and grip. Contact forces include physical hitch load transfer; trailer resistance is
transmitted through the joints. Trailer mass is not simply added to tractor tyre load.
The rolling-speed reference follows the support plane on slopes. Traction control cannot
command rollback when the requested direction is uphill, or release a zero-speed hold by
tracking downhill wheel speed.

The Machines drive panel shows the configured power and wheel torque ceilings, powered and
loaded wheel counts, available motor torque, and power limiting. Available torque is a ceiling,
not a measurement of applied torque. No torque/power values are hardcoded by machine name.

## Authoring

List driven rolling joints in `gearbox:machine:role:poweredWheelJoints`, not steering joints.
Keep free-rolling wheels in `passiveWheelJoints`; a wheel must not be in both roles. Author
torque and power on the drive controller. Without these values, the legacy grip-only limit
remains, so a realistic power rating must be supplied for a calibrated towing vehicle.
A zero command coasts by default; servo-geared robots author `zeroCommand = "brake"` so the
wheel motors stop the machine before the parking hold takes over.

Use wheel-output torque, not engine-shaft torque. Torque multiplication, gearing efficiency
and power available after auxiliary loads are authoring inputs in this model. Engine RPM,
clutch behavior, gear changes, deformable soil and differential locking are not simulated here.
The trailer must have physical mass, freely rolling wheels, released brakes and a real hitch.

## Local Oxbo upgrade

`~/data/code/OUSD/machines/usd/oxbo_harvester.usdz` now authors all six roll joints as powered,
no passive wheels, a 35,000 N·m per-wheel ceiling and a 291 kW drive-power budget. This is a
custom simulation upgrade. [Oxbo's EPD540e brochure](https://www.oxbo.com/app/uploads/2022/08/EPD540e_ENG.pdf)
lists four-wheel drive and 291 kW engine output, not a factory six-wheel-drive specification
or 35,000 N·m wheel torque. The chosen drive budget does not subtract auxiliary power losses.

The original package is preserved at
`~/data/code/OUSD/machines/.backups/20260914-6wd/oxbo_harvester.usdz`.
Geometry and articulation are unchanged. The Blender source is unchanged; exporting it again
requires reapplying this drivetrain metadata. The package remains self-contained and is
repacked with OpenUSD's `usdzip`, not a generic compressed ZIP writer.

The contact load of a wheel with a tyre element is Molla's tyre output: its normal load and grip
force. Other wheels read the normal impulses of their Molla contact manifolds.
This is a simulation model; no real-machine safe slope or towing rating is claimed.

## Rear-wheel observation

During manual driving on the mixed-field terrain, one Oxbo rear wheel had zero
solved grip while its shaft tracked approximately 2.58 rad/s against a 2.59 rad/s
target. A parked unloaded rear wheel measured about 0.00005 rad/s. The drive panel
correctly reported five loaded wheels rather than counting its braking torque as contact.
The asset has vertical suspension joints on the middle wheels, but rigid rear
steering mounts. Rear contact can still be lost on uneven terrain; axle articulation
or suspension authoring is a separate remaining asset task, not solved by shaft control.
