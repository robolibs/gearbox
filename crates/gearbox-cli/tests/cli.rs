//! End-to-end: the `gearbox` binary against an in-process fake host, with
//! every file the CLI touches redirected into a temp directory.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use gearbox_api::fake::FakeHost;
use gearbox_api::registry::{self, RegistryEntry};

static COUNTER: AtomicU32 = AtomicU32::new(0);

#[test]
fn tyre_pressure_rejects_invalid_values_and_conflicting_selectors() {
    let env = Env::start(&["tractor"]);
    for bar in ["NaN", "inf", "0"] {
        let (code, _, err) = env.gearbox(&["machine", "tyre-pressure", bar, "--machine", "tractor"]);
        assert_eq!(code, 2, "{err}");
        assert!(err.contains("finite positive"), "{err}");
    }
    for args in [
        vec!["machine", "tyre-pressure", "1.8", "--axle", "0"],
        vec!["machine", "tyre-pressure", "1.8", "--axle", "1", "--wheel", "left"],
    ] {
        let (code, _, err) = env.gearbox(&args);
        assert_eq!(code, 2, "{err}");
    }
}

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
        let out = Command::new(env!("CARGO_BIN_EXE_gearbox-cli"))
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

    /// Same fake host, but with no registry entry visible and no instance
    /// named — proves a code path needs no instance at all, not just that
    /// one happens to be resolvable.
    fn gearbox_no_instance(&self, args: &[&str]) -> (i32, String, String) {
        let empty_run = self.dir.join("run-empty");
        std::fs::create_dir_all(&empty_run).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_gearbox-cli"))
            .args(args)
            .env("GEARBOX_REGISTRY_DIR", empty_run)
            .env("XDG_CONFIG_HOME", self.dir.join("config"))
            .env("XDG_STATE_HOME", self.dir.join("state"))
            .env("AGENTIO_KEYS_DIR", self.dir.join("keys"))
            .env_remove("GEARBOX_INSTANCE")
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
    assert_eq!(v[0]["machine_id"], "oxbo");

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

/// `peer sub` decodes any full topic path generically, with no per-topic
/// code — this is what makes `state` show up correctly even though it's
/// read through the untyped path, not `machine state`'s typed one.
#[test]
fn peer_sub_decodes_a_topic_by_full_path() {
    let env = Env::start(&["oxbo"]);
    let (code, out, err) = env.gearbox(&["peer", "sub", "/machines/oxbo/state", "-n", "1"]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v.get("odom").is_some(), "{out}");

    let (code, _, err) = env.gearbox(&["peer", "sub", "/machines/oxbo/nosuchtopic", "-n", "1"]);
    assert_ne!(code, 0, "{err}");

    // Every topic is spelled as a topic — a bare word with no leading `/`
    // is rejected, not silently treated as a leaf.
    let (code, _, err) = env.gearbox(&["peer", "sub", "state", "-n", "1"]);
    assert_ne!(code, 0, "bare word without a leading / should be rejected: {err}");
}

/// `peer list` returns the same topics directly; `peer sub` on a path with
/// no owner of its own but live topics under it lists those instead of
/// erroring — `/machines/oxbo` isn't a topic, `/machines/oxbo/state` is. A
/// path with nothing under it either way still errors.
#[test]
fn peer_list_and_sub_on_a_prefix_lists_its_children() {
    let env = Env::start(&["oxbo"]);
    let (code, out, err) = env.gearbox(&["peer", "list", "/machines/oxbo", "--json"]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let topics: Vec<&str> = v.as_array().unwrap().iter()
        .map(|row| row["topic"].as_str().unwrap())
        .collect();
    assert!(topics.contains(&"/machines/oxbo/state"), "{out}");
    assert!(topics.contains(&"/machines/oxbo/odom"), "{out}");
    assert!(topics.iter().all(|t| t.starts_with("/machines/oxbo/")), "{out}");

    let (code, out, err) = env.gearbox(&["peer", "sub", "/machines/oxbo", "--json"]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let topics: Vec<&str> = v.as_array().unwrap().iter()
        .map(|row| row["topic"].as_str().unwrap())
        .collect();
    assert!(topics.contains(&"/machines/oxbo/state"), "{out}");

    let (code, _, err) = env.gearbox(&["peer", "sub", "/machines/nosuchmachine", "-n", "1"]);
    assert_ne!(code, 0, "a prefix with nothing under it should still error: {err}");
}

/// `--did` dials the machine's own endpoint directly (agentio's `by_id`
/// escape hatch), skipping the host's directory entirely — real proof it
/// works, not just that it compiles.
#[test]
fn peer_sub_dials_the_machines_own_did_directly() {
    let env = Env::start(&["oxbo"]);
    let (code, out, err) = env.gearbox(&["machine", "list", "--json"]);
    assert_eq!(code, 0, "{err}");
    let list: serde_json::Value = serde_json::from_str(&out).unwrap();
    let did = list[0]["did"].as_str().expect("machine did").to_string();

    let (code, out, err) = env.gearbox(&[
        "peer",
        "sub",
        "/machines/oxbo/state",
        "--did",
        &did,
        "-n",
        "1",
    ]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v.get("odom").is_some(), "{out}");

    let (code, _, err) = env.gearbox(&[
        "peer",
        "sub",
        "/machines/oxbo/state",
        "--did",
        "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK",
        "-n",
        "1",
    ]);
    assert_ne!(code, 0, "wrong did should not silently resolve: {err}");
}

/// `--did` needs no instance at all — real proof, not just that one
/// happens to resolve: an empty registry and no `GEARBOX_INSTANCE` still
/// reaches the machine.
#[test]
fn peer_sub_by_did_needs_no_instance() {
    let env = Env::start(&["oxbo"]);
    let (code, out, err) = env.gearbox(&["machine", "list", "--json"]);
    assert_eq!(code, 0, "{err}");
    let list: serde_json::Value = serde_json::from_str(&out).unwrap();
    let did = list[0]["did"].as_str().expect("machine did").to_string();

    let (code, out, err) = env.gearbox_no_instance(&[
        "peer",
        "sub",
        "/machines/oxbo/state",
        "--did",
        &did,
        "-n",
        "1",
    ]);
    assert_eq!(code, 0, "no instance registered, yet --did worked: {err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v.get("odom").is_some(), "{out}");

    // Same environment, no --did: this one really does need an instance.
    let (code, _, err) = env.gearbox_no_instance(&["machine", "list"]);
    assert_ne!(code, 0, "should fail without any instance: {err}");
}

/// `access grant` reaches a running instance immediately, not just its
/// next launch — the host's live `allowed_peers()` list actually changes.
#[test]
fn access_grant_applies_to_the_live_instance() {
    let env = Env::start(&["oxbo"]);
    let (code, out, err) = env.gearbox(&["access", "identity", "new", "guest", "--json"]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let did = v["did"].as_str().expect("guest did").to_string();

    let (code, out, err) = env.gearbox(&["instance", "info", "--json"]);
    assert_eq!(code, 0, "{err}");
    let before: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(
        !before["props"]["allowed"]
            .as_str()
            .unwrap_or_default()
            .contains(&did),
        "guest should not be allowed yet: {before}"
    );

    let (code, out, err) = env.gearbox(&["access", "grant", &did, "--json"]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["live"], true, "should have reached the running instance: {v}");

    let (code, out, err) = env.gearbox(&["instance", "info", "--json"]);
    assert_eq!(code, 0, "{err}");
    let after: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(
        after["props"]["allowed"]
            .as_str()
            .unwrap_or_default()
            .contains(&did),
        "guest should be allowed live, no restart: {after}"
    );
}

/// With several instances running and none named, the CLI picks one
/// (alphabetically first) instead of erroring.
#[test]
fn no_instance_given_picks_the_first_of_several() {
    let a = Env::start(&[]);
    let b = Env::start(&[]);
    let names = {
        let mut n = [a.name.clone(), b.name.clone()];
        n.sort();
        n
    };
    // Point b's registry lookups at a's directory too, so both entries are
    // visible from one process without a real shared registry dir.
    for entry in std::fs::read_dir(b.dir.join("run")).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), a.dir.join("run").join(entry.file_name())).unwrap();
    }

    let out = Command::new(env!("CARGO_BIN_EXE_gearbox-cli"))
        .args(["instance", "info", "--json"])
        .env("GEARBOX_REGISTRY_DIR", a.dir.join("run"))
        .env("XDG_CONFIG_HOME", a.dir.join("config"))
        .env("XDG_STATE_HOME", a.dir.join("state"))
        .env("AGENTIO_KEYS_DIR", a.dir.join("keys"))
        .env_remove("GEARBOX_INSTANCE")
        .env("HOME", &a.dir)
        .output()
        .expect("run gearbox");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["props"]["name"], "fake");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(&names[0]),
        "should say which instance it picked: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn no_instance_is_exit_3() {
    let dir = std::env::temp_dir().join(format!("gearbox-cli-empty-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("run")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_gearbox-cli"))
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

#[test]
fn attach_and_detach_through_the_master() {
    let env = Env::start(&["tractor", "trailer"]);
    let (code, out, err) = env.gearbox(&["machine", "tools", "list", "tractor"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("nothing attached"), "{out}");

    let (code, out, err) = env.gearbox(&[
        "machine",
        "tools",
        "attach",
        "trailer",
        "--machine",
        "tractor",
        "--teleport",
        "--json",
    ]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["slave"], "trailer");

    let (code, out, err) = env.gearbox(&["machine", "tools", "list", "tractor", "--json"]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v[0]["props"]["slave"], "trailer");

    let (code, out, err) = env.gearbox(&["machine", "links", "tractor", "--json"]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let names: Vec<&str> = v["links"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|l| l["props"]["name"].as_str())
        .collect();
    assert!(names.contains(&"trailer/base_link"), "{names:?}");

    let (code, _, err) = env.gearbox(&["machine", "move", "trailer", "--for", "200ms"]);
    assert_eq!(code, 4, "an attached slave refuses commands: {err}");
    assert!(err.contains("attached"), "{err}");

    let (code, _, err) = env.gearbox(&["machine", "tools", "detach", "trailer", "--machine", "tractor"]);
    assert_eq!(code, 0, "{err}");
    let (code, out, _) = env.gearbox(&["machine", "tools", "list", "tractor", "--json"]);
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "[]");
    let (code, _, err) = env.gearbox(&["machine", "move", "trailer", "--for", "200ms"]);
    assert_eq!(code, 0, "detached slave drives again: {err}");
}

#[test]
fn scene_tree_and_link_values_of_a_fake_machine() {
    let env = Env::start(&["oxbo"]);
    let (code, out, err) = env.gearbox(&["machine", "links", "oxbo", "--flat"]);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("ELEMENT") && out.contains("base_link"),
        "{out}"
    );
    let (code, _, err) = env.gearbox(&[
        "machine",
        "set-value",
        "nope",
        "position",
        "1",
        "--machine",
        "oxbo",
    ]);
    assert_eq!(code, 2, "unknown link is a usage error: {err}");
    let (code, _, err) = env.gearbox(&[
        "machine",
        "set-value",
        "base_link",
        "position",
        "1",
        "--machine",
        "oxbo",
    ]);
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = env.gearbox(&["scene", "tree"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("oxbo") && out.contains("base_link"), "{out}");
}
