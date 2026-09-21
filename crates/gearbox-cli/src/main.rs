//! Dev/test entry point only — the shipped binary is the merged `gearbox`
//! built from `bin/gearbox`, which calls straight into `gearbox_cli::run()`.
//! This one exists so `cargo test -p gearbox-cli` has something to spawn
//! without pulling in the simulator's entire Bevy/rendering dependency
//! graph just to exercise CLI dispatch logic.

fn main() {
    std::process::exit(gearbox_cli::run());
}
