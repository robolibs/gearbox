This directory vendors only the `bevy_openusd` crates Gearbox uses:

- `crates/usd_bevy`
- `crates/usd_schema`
- `crates/usd_rapier`

The upstream viewer app and examples were intentionally not vendored because
they still depend on older viewer helper crates. Gearbox owns the Bevy/Mara
viewer integration locally.
