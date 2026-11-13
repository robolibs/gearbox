#[path = "../tests/support/draw_data_wire.rs"]
mod fixtures;

fn main() {
    println!(
        "{}",
        serde_json::to_string_pretty(&fixtures::fixture()).unwrap()
    );
}
