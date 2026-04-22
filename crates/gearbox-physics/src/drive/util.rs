//! Shared helpers for drive controllers.

/// Per-wheel Ackermann correction.
///
/// Given a commanded steer angle (clamped to `max_steer`) and a wheel's
/// lateral offset `wheel_x` on a vehicle with wheelbase `wheelbase`,
/// return the wheel's actual steering angle such that every wheel's
/// individual heading converges on a single turn centre (no tyre scrub
/// across an axle).
///
/// Worked in absolute values and then signed by the input direction,
/// so left and right turns behave symmetrically. (An earlier version
/// used the signed `R_c = wheelbase / tan(δ_c)` directly in `atan2`,
/// which returned angles > π/2 for right turns — those got clamped to
/// `max_steer` for BOTH wheels, so any right-stick input pegged the
/// fronts to full lock instantly. The abs-then-signed form below has
/// identical behaviour left vs right.)
pub fn ackermann_steer(input: f32, max_steer: f32, wheel_x: f32, wheelbase: f32) -> f32 {
    if input.abs() < 1e-6 || max_steer.abs() < 1e-6 || wheelbase <= 1e-3 {
        return input * max_steer;
    }
    // `max_steer` can be NEGATIVE — that's how some presets
    // (oxbo rear axle) encode "crab-steer, opposite direction from
    // the front". So the final output sign follows `input * max_steer`,
    // not `input` alone.
    let sign = (input * max_steer).signum();
    let abs_max = max_steer.abs();
    // Central turn angle (positive magnitude).
    let abs_delta_c = (input * max_steer).abs().min(abs_max);
    // Central turn radius — distance from rear axle to turn centre.
    let abs_r_c = wheelbase / abs_delta_c.tan();
    // Wheel's lateral distance from the turn-centre line (signed —
    // the inside wheel of a very tight turn can flip sign). We use
    // the magnitude for the `atan2` since the wheel's steering angle
    // is always < π/2 in magnitude.
    let r_w_lateral = wheel_x + sign * abs_r_c;
    let delta_w_mag = wheelbase.atan2(r_w_lateral.abs());
    let delta_signed = delta_w_mag * sign;
    delta_signed.clamp(-abs_max, abs_max)
}
