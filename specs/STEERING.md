# Ackermann steering and wheel-speed targets

The shared solver is `bin/gearbox/src/controller/steering.rs`. It handles the
Ackermann and counter-steering tokens; skid steering, crab and parallel mappings
remain separate. It uses the same no-lateral-slip kinematic constraint described
in [ROS 2 Control's steering library](https://control.ros.org/master/doc/ros2_controllers/steering_controllers_library/doc/userdoc.html).

Chassis coordinates are X left, Y back, Z up. For curvature `k`, pivot axle `p`,
and wheel centre `(x, y)`, the rolling vector per unit forward speed is:

```
forward = 1 - k*x
lateral = k*(p-y)
steer = atan2(lateral, forward)
wheel_omega = speed * hypot(forward, lateral) / tyre_radius
```

Steering targets use joint pivots; drive targets use tyre centres. All targets
share one curvature. A limit on any steering joint reduces that curvature for
the whole machine rather than independently clipping wheel angles.
Traction control and torque/power ceilings subsequently limit motor effort.
Fallback steering servos size stiffness for 0.5° error at their torque ceiling,
and damping for 30°/s at that ceiling. Authored force-based position gains remain
in effect when supplied; these rate/error values are gain references, not hard motion limits.
For resolved geometric steering, unauthored servos update from solved tyre loads
each frame, with a 25% scrub margin and the 100 kNm fallback ceiling. Original USD
drive metadata distinguishes authored motors from runtime-generated motors.
This is differential speed allocation, not a mechanical differential or tyre
slip-angle simulation; steering transients and terrain can still produce slip.

## Oxbo asset

`/home/bresilla/data/code/OUSD/machines/usd/oxbo_harvester.usdz` has front,
middle and rear axle Y positions -2.5, -0.9 and 2.36 m. The fixed middle axle
sets the pivot line. Front/rear lever arms are 1.6 and -3.26 m, so fixed
degree offsets and constant angle multipliers do not give exact rolling geometry.
Those overrides were removed. Existing physical joint limits remain ±16° front,
±29° rear; the controller ceiling is 29°. Six powered wheels and the earlier
35,000 Nm/wheel, 291 kW custom simulation upgrade are retained.

The pre-steering package is backed up in
`/home/bresilla/data/code/OUSD/machines/.backups/20260914-ackermann/`.
The Blender source was not changed; re-exporting it requires preserving this metadata.

## Validation

Regression cases cover mirrored inner/outer angles, Oxbo axle geometry, common
curvature saturation, signed wheel speeds and small normalized steering inputs.
They are authored but not run under the current no-automated-tests constraint.
`GEARBOX_STEERING_DEBUG=1` logs solved curvature, steering targets/actual joint
angles, wheel spin targets/actual rates, solved grip and approximate terrain
clearance for manual preview inspection. The clearance estimate is not a contact test.

The manual 1.5 m/s, 0.05 rad/s Oxbo turn reached approximately 1.43 m/s and
0.04–0.05 rad/s. Front targets were 3.18°/2.93°, rear -6.46°/-5.97°.
With the old servo, observed steering errors reached roughly 5–7° under load;
the torque-scaled servo reduced sampled settled errors to below 0.8° on that pass.
This is a slow-turn observation, not proof of zero slip on every slope or trailer load.

With the per-frame load-aware fallback servo, a subsequent 1.5 m/s, 0.15 rad/s
manual turn measured roughly 1.49–1.52 m/s and 0.15 rad/s. One settled sample had
front targets/actuals 10.32°/10.29° and 8.12°/7.99°, rear -20.33°/-20.57° and
-16.23°/-16.23°. The unloaded outside rear wheel spun at 2.70 rad/s against a
2.73 rad/s target. Logs: `/tmp/gearbox-steering-ready.log` and
`/tmp/gearbox-steering-final-turn.log`; screenshot `/tmp/gearbox-steering-final.png`.
