//! End-to-end: the `gearbox` binary against an in-process fake host, with
//! every file the CLI touches redirected into a temp directory.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use gearbox_api::fake::FakeHost;
use gearbox_api::registry::{self, RegistryEntry};

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Env {
    dir: PathBuf,
    _host: FakeHost,
    name: String,
}

impl Env {
    fn start(machines: &[&str]) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("gearbox-cli-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(dir.join("run")).unwrap();
        let name = format!("clitest{n}");
        let host = FakeHost::start(&name, machines).expect("fake host");
        let entry = RegistryEntry {
            name: name.clone(),
            did: host.did.clone(),
            addr: host.addr_hex.clone(),
            pid: std::process::id(),
            started: registry::now_rfc3339(),
            version: "fake".into(),
            log: None,
        };
        // Written by hand: tests share one process, so the env override the
        // registry module reads cannot differ per test.
        std::fs::write(
            dir.join("run").join(format!("{name}.json")),
            serde_json::to_string_pretty(&entry).unwrap(),
        )
        .expect("registry write");
        Self {
            dir,
            _host: host,
            name,
        }
    }

    fn gearbox(&self, args: &[&str]) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_gearbox"))
            .args(args)
            .env("GEARBOX_REGISTRY_DIR", self.dir.join("run"))
            .env("XDG_CONFIG_HOME", self.dir.join("config"))
            .env("XDG_STATE_HOME", self.dir.join("state"))
            .env("AGENTIO_KEYS_DIR", self.dir.join("keys"))
            .env("GEARBOX_INSTANCE", &self.name)
            .env("HOME", &self.dir)
            .output()
            .expect("run gearbox");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn instance_and_scene_flow() {
    let env = Env::start(&[]);
    let (code, out, err) = env.gearbox(&["instance", "list", "--json"]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v[0]["name"], env.name);
    assert_eq!(v[0]["alive"], true);

    let (code, out, err) = env.gearbox(&["instance", "info", "--json"]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["props"]["name"], "fake");

    let (code, out, err) = env.gearbox(&[
        "spawn",
        "usd",
        "markers/bale.usdz",
        "--id",
        "bale_1",
        "--at",
        "1",
        "0",
        "2",
        "--quiet",
    ]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out.trim(), "bale_1");

    let (code, out, err) = env.gearbox(&["scene", "list", "--json"]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["props"]["id"], "bale_1");
    assert_eq!(v[0]["x"], 1.0);

    let (code, _, err) = env.gearbox(&["scene", "pause"]);
    assert_eq!(code, 0, "{err}");
    let (code, out, _) = env.gearbox(&["scene", "status", "--json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["paused"], true);

    let (code, _, err) = env.gearbox(&["select", "object", "bale_1"]);
    assert_eq!(code, 0, "{err}");
    let (code, out, _) = env.gearbox(&["select", "show", "--json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["context"]["id"], "bale_1");

    let (code, _, err) = env.gearbox(&["clear", "all", "-y"]);
    assert_eq!(code, 0, "{err}");
    let (code, out, _) = env.gearbox(&["scene", "list", "--json"]);
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "[]");

    let (code, out, err) = env.gearbox(&["api", "call", "/gearbox/info", "--json"]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["props"]["name"], "fake");

    let (code, out, err) = env.gearbox(&["api", "schema", "usd_load", "--json"]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["name"], "gearbox.usd_load.v1");
}

#[test]
fn machine_move_and_busy() {
    let env = Env::start(&["oxbo"]);
    let (code, out, err) = env.gearbox(&["machine", "list", "--json"]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v[0]["namespace"], "oxbo");

    let (code, out, err) = env.gearbox(&[
        "machine",
        "move",
        "oxbo",
        "--forward",
        "2",
        "--for",
        "600ms",
        "--json",
    ]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["z"].as_f64().unwrap() > 0.3, "moved forward: {v}");

    let (code, _, err) = env.gearbox(&["machine", "controllers", "oxbo"]);
    assert_eq!(code, 0, "{err}");

    let (code, out, err) = env.gearbox(&["machine", "links", "oxbo", "--json"]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["links"][0]["props"]["name"], "base_link");
    assert_eq!(v["links"][1]["props"]["parent"], "base_link");
    assert_eq!(v["derived"], true);

    let (code, out, err) = env.gearbox(&["machine", "links", "oxbo"]);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("base_link") && out.contains("wheel_left"),
        "{out}"
    );

    let (code, _, err) = env.gearbox(&["machine", "move", "nosuch", "--for", "100ms"]);
    assert_ne!(code, 0, "{err}");

    let (code, _, err) = env.gearbox(&["access", "list"]);
    assert_eq!(code, 0, "{err}");
}

#[test]
fn no_instance_is_exit_3() {
    let dir = std::env::temp_dir().join(format!("gearbox-cli-empty-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("run")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_gearbox"))
        .args(["scene", "status"])
        .env("GEARBOX_REGISTRY_DIR", dir.join("run"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("AGENTIO_KEYS_DIR", dir.join("keys"))
        .env_remove("GEARBOX_INSTANCE")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let _ = std::fs::remove_dir_all(&dir);
}
