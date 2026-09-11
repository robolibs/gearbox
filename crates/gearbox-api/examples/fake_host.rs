//! Run the in-process fake host as a standalone process, registered under
//! the given instance name, so Python and CLI clients can be tested
//! without the simulator:
//!
//! ```text
//! cargo run -p gearbox-api --no-default-features --example fake_host -- [name] [machine ns]...
//! ```

use gearbox_api::fake::FakeHost;
use gearbox_api::registry::{self, RegistryEntry};

fn main() {
    let mut args = std::env::args().skip(1);
    let name = args.next().unwrap_or_else(|| "fake".to_string());
    let machines: Vec<String> = args.collect();
    let machines: Vec<&str> = if machines.is_empty() {
        vec!["oxbo"]
    } else {
        machines.iter().map(String::as_str).collect()
    };
    let host = FakeHost::start(&name, &machines).expect("fake host");
    let entry = RegistryEntry {
        name: name.clone(),
        did: host.did.clone(),
        addr: host.addr_hex.clone(),
        pid: std::process::id(),
        started: registry::now_rfc3339(),
        version: "fake".to_string(),
        log: None,
    };
    let path = registry::write(&entry).expect("registry write");
    println!(
        "fake host `{name}`\n  did      {}\n  addr     {}\n  registry {}\n  machines {:?}",
        host.did,
        host.addr_hex,
        path.display(),
        machines
    );
    println!("type `quit` + Enter, or kill the process, to stop");
    let mut buf = String::new();
    loop {
        buf.clear();
        match std::io::stdin().read_line(&mut buf) {
            Ok(0) => std::thread::sleep(std::time::Duration::from_secs(3600)),
            Ok(_) if buf.trim() == "quit" => break,
            Ok(_) => {}
            Err(_) => std::thread::sleep(std::time::Duration::from_secs(3600)),
        }
    }
    registry::remove(&name);
    drop(host);
}
