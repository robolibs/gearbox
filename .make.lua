-- gearbox's build, as recipes. This replaced the Makefile; there is no other.
--
--   make            the recipes, with what each of them says it does
--   make build      everything in the workspace (release)
--   make run        the simulator, through the CLI
--   make test       the suite
--
-- At an oslo prompt in this directory `make` is enough; everywhere else it is `oslo make`.
-- The dev shell's toolchain comes from `.env.lua`'s `nix_develop({ impure = true })`, so
-- recipes call `cargo` directly rather than wrapping every command in `nix develop -c`.
-- NVIDIA/`nixVulkan` detection lives in `.env.lua` (it decides the flake's own devshell
-- package selection, so it has to run before `nix_develop()` does, once per
-- directory-entry — see that file). This file only re-detects the Wayland socket per
-- launch, since that (unlike NVIDIA presence) can genuinely change mid-session
-- (compositor restart).

local make = oslo.make

-- `run`/`sim` re-detect the Wayland socket fresh, every launch, instead of trusting
-- whatever WAYLAND_DISPLAY a shell happened to inherit — a compositor restart hands out a
-- fresh randomized socket name, and a long-lived shell keeps the old (now-dead) one
-- exported forever, which would silently fall back to XWayland.
--
-- Deliberately does NOT use the names `BACKEND`/`WAYLAND_DISPLAY`/`DISPLAY` for its own
-- computed values: on at least one machine those names came back `readonly` in the
-- interactive shell, so `unset`/`export` on them silently no-ops and this whole detection
-- block would compute the right answer and then have it thrown away. `_gb_*` names can't
-- collide with anything set before this script runs.
local function fresh_display_env(cmd)
  return ([[
_gb_xdg_runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
_gb_wl=""
if [ -n "${WAYLAND_DISPLAY:-}" ] && [ -S "$_gb_xdg_runtime/$WAYLAND_DISPLAY" ]; then
  _gb_wl="$WAYLAND_DISPLAY"
elif [ -S "$_gb_xdg_runtime/wayland-0" ]; then
  _gb_wl="wayland-0"
else
  _gb_wl=$(ls -t "$_gb_xdg_runtime"/wayland-* 2>/dev/null | grep -v '\.lock$' | head -n1 | xargs -r basename)
fi
if [ -n "$_gb_wl" ]; then
  _gb_backend=wayland
  export WAYLAND_DISPLAY="$_gb_wl"
  unset DISPLAY
else
  _gb_backend=x11
  export DISPLAY="${DISPLAY:-:1}"
  unset WAYLAND_DISPLAY
fi
%s
]]):format(cmd)
end

local function need(tool, why)
  assert(oslo.run{ "sh", "-c", "command -v " .. tool, capture = true }.ok, why)
end

-- name = ... from bin/gearbox/Cargo.toml, the one place every tool here reads it from.
local function project_name()
  local content = oslo.fs.read("bin/gearbox/Cargo.toml") or ""
  local name = content:match('\nname%s*=%s*"([^"]+)"') or content:match('^name%s*=%s*"([^"]+)"')
  assert(name, "package name not found in bin/gearbox/Cargo.toml")
  return name
end

local NAME = project_name()

local function forwarded_args(a)
  assert(a.args == nil or type(a.args) == "string",
    "use --args='--planet mars --load /path/to/rover.usdz' (with = before dash-prefixed arguments)")
  return a.args or ""
end

-- One build, and it is the one `run`, `sim` and `cli` launch. There used to be
-- a second recipe for the release binary, which meant `make build` could leave
-- the binary on PATH untouched and hours old while everything looked rebuilt.
make.recipe{
  name = "build",
  desc = "everything in the workspace (release) — every crate, lib and bin",
  run = function() sh.cargo("build", "--release") end,
}
make.alias("b", "build")
make.recipe{ name = "build-dev", desc = "debug simulator and command-line client",
             run = function() sh.cargo("build", "-p", "gearbox-sim", "-p", "gearbox-cli") end }

-- Debug sim with per-system Chrome tracing; run it with GEARBOX_TRACE=file.json.
make.recipe{
  name = "build-profile",
  desc = "debug binary built with the profile feature (GEARBOX_TRACE=file.json to use it)",
  run = function()
    sh.cargo("build", "-p", "gearbox-sim", "--bin", "gearbox",
                       "--features", "gearbox-sim/profile")
  end,
}

make.recipe{
  name = "build-release-profile",
  desc = "release binary with per-system Chrome tracing",
  run = function()
    sh.cargo("build", "--release", "-p", "gearbox-sim", "--bin", "gearbox",
                       "--features", "gearbox-sim/profile")
  end,
}

make.recipe{ name = "compile", desc = "clean, then build", deps = { "clean", "build" } }
make.alias("c", "compile")

make.recipe{
  name = "run",
  desc = "launch the simulator through the CLI (fresh Wayland detection, nixVulkan wrapper)",
  params = { { "--args", desc = "gearbox run arguments; use --args='--planet mars ...'" } },
  deps = { "build" },
  run = function(a)
    sh.sh("-c", fresh_display_env(
      ("env WINIT_UNIX_BACKEND=$_gb_backend nixVulkan target/release/gearbox run %s"):format(forwarded_args(a))))
  end,
}
make.alias("r", "run")

make.recipe{
  name = "sim",
  desc = "launch the sim directly, bypassing `gearbox run`'s instance management",
  params = { { "--args", desc = "USD scenes to load, passed through to `gearbox launch`" } },
  deps = { "build" },
  run = function(a)
    sh.sh("-c", fresh_display_env(
      ("env WINIT_UNIX_BACKEND=$_gb_backend nixVulkan target/release/gearbox launch %s"):format(forwarded_args(a))))
  end,
}

make.recipe{
  name = "cli",
  desc = "run the gearbox CLI: make cli --args 'instance list'",
  params = { { "--args", desc = "arguments passed through to the CLI" } },
  deps = { "build" },
  run = function(a)
    sh.sh("-c", ("target/release/gearbox %s"):format(forwarded_args(a)))
  end,
}

make.recipe{ name = "test", desc = "run all tests",
             run = function() sh.cargo("test", "--all-targets") end }
make.alias("t", "test")
make.recipe{ name = "test-launch-args", desc = "check Oslo launch argument forwarding without starting a simulator",
             run = function() sh.bash("scripts/test-launch-args.sh") end }
make.recipe{ name = "test-planets", desc = "test planetary frames and terrain",
             run = function()
               sh.cargo("test", "-p", "gearbox-globe")
               sh.cargo("test", "-p", "bevy_weather")
               sh.cargo("test", "-p", "gearbox-fields", "--lib")
               sh.cargo("test", "-p", "gearbox-cli", "planet_tests")
               sh.cargo("test", "-p", "gearbox-planet", "--lib")
               sh.cargo("test", "-p", "gearbox-planet", "--test", "surface_handoff")
               sh.cargo("test", "-p", "gearbox-sim", "--bin", "gearbox", "planet_tests")
             end }
make.recipe{ name = "test-planet-gpu", desc = "compare production globe shader heights with local terrain on Vulkan",
             run = function() sh.nixVulkan("cargo", "test", "-p", "gearbox-planet", "--test", "regolith_gpu", "--", "--ignored", "--nocapture") end }
make.recipe{ name = "test-planet-geometry", desc = "check global and local planetary geometry",
             run = function() sh.cargo("test", "-p", "gearbox-globe") end }
make.recipe{ name = "test-render-pages", desc = "test sparse GPU buffer page boundaries",
             run = function() sh.cargo("test", "-p", "bevy_render", "planet_sparse_page_tests") end }
make.recipe{ name = "test-planet-gravity", desc = "measure native gravity after imported USD scenes on each planet",
             run = function() sh.cargo("test", "-p", "gearbox-sim", "--bin", "gearbox", "planet_gravity_all_bodies", "--", "--nocapture") end }
make.recipe{ name = "test-sim", desc = "run the simulator's tests matching --filter=NAME",
             params = { { "--filter", desc = "test name filter" } },
             run = function(a)
               assert(type(a.filter) == "string", "--filter=NAME is required")
               sh.cargo("test", "-p", "gearbox-sim", "--bin", "gearbox", a.filter)
             end }
make.recipe{ name = "test-hitch", desc = "test grouped hitch control and joint compliance",
             run = function() sh.cargo("test", "-p", "gearbox-sim", "--bin", "gearbox", "services::") end }
make.recipe{ name = "test-tracked-machine", desc = "test the imported GEARBOX_TRACK_ASSET through native tracked control",
             run = function() sh.cargo("test", "-p", "gearbox-sim", "real_tracked_machine", "--", "--ignored", "--nocapture") end }
make.recipe{ name = "test-fem", desc = "test native FEM scheduling",
             run = function() sh.cargo("test", "-p", "gearbox-sim", "--bin", "gearbox", "physics::fem") end }
make.recipe{ name = "test-fem-machine", desc = "validate FEM binding against GEARBOX_TRACK_ASSET",
             run = function() sh.cargo("test", "-p", "gearbox-sim", "--bin", "gearbox", "ceol_fem_machine_binding", "--", "--ignored", "--nocapture") end }
make.recipe{ name = "test-fem-visuals", desc = "validate CPU-prepared GPU visual bindings against GEARBOX_TRACK_ASSET",
             run = function() sh.cargo("test", "-p", "gearbox-sim", "--bin", "gearbox", "ceol_fem_visual_binding", "--", "--ignored", "--nocapture") end }
make.recipe{ name = "export-fem-mass", desc = "export GEARBOX_TRACK_ASSET nodal properties to GEARBOX_FEM_MASS_EXPORT",
             run = function() sh.cargo("test", "-p", "gearbox-sim", "--bin", "gearbox", "export_ceol_fem_mass_properties", "--", "--ignored", "--nocapture") end }
make.recipe{ name = "export-fem-energy", desc = "export CPU FEM inertia reference to GEARBOX_FEM_ENERGY_EXPORT",
             run = function() sh.cargo("test", "-p", "gearbox-sim", "--bin", "gearbox", "export_ceol_fem_energy_reference", "--", "--ignored", "--nocapture") end }
make.recipe{ name = "test-fem-gpu", desc = "test native FEM on a GPU with GEARBOX_TRACK_ASSET",
             params = { { "--filter", desc = "test name filter (default: physics::fem)" } },
             run = function(a) sh.nixVulkan("cargo", "test", "-p", "gearbox-sim", "--bin", "gearbox", a.filter or "physics::fem", "--", "--ignored", "--nocapture", "--test-threads=1") end }

-- Robot structure and control contract (plans/001). Each gate prints cargo's output, fails on a
-- failing run, and fails a run that selected fewer tests than its inventory names.
local function contract_tests(label, args, required)
  local cmd = { "cargo", "test" }
  for _, a in ipairs(args) do cmd[#cmd + 1] = a end
  cmd.capture = true
  cmd.capture_err = true
  local r = oslo.run(cmd)
  io.write(r.err or "", r.out or "")
  assert(r.ok, label .. ": cargo test failed")
  local passed = 0
  for n in (r.out or ""):gmatch("test result: ok%. (%d+) passed") do passed = passed + tonumber(n) end
  assert(passed > 0, label .. ": no test selected")
  for _, name in ipairs(required) do
    assert((r.out or ""):find(name .. " ... ok", 1, true), label .. ": required test did not pass: " .. name)
  end
  print(("%s: %d tests passed"):format(label, passed))
end

local function machine_contract_gate()
  contract_tests("machine contract", { "-p", "gearbox-sim", "--bin", "gearbox", "machine_model::" },
    { "machine_model::tests::structure_fixture_inventory_is_complete_and_passes",
      "machine_model::tests::capability_evidence_names_real_tests" })
end

local function joint_control_gate()
  contract_tests("joint control", { "-p", "gearbox-sim", "--bin", "gearbox", "--", "joint_control::", "physics::commit_tests::" },
    { "joint_control::tests::command_inventory_passes", "joint_control::schedule_tests::schedule_inventory_passes",
      "physics::commit_tests::physics_command_inventory_passes" })
end

make.recipe{
  name = "test-machine-contract",
  desc = "structural compiler fixtures S01-S23 and the manifest (plans/001)",
  run = machine_contract_gate,
}

make.recipe{
  name = "test-joint-control",
  desc = "ownership, units, limits, targets and scheduler of the joint command contract (plans/001)",
  run = joint_control_gate,
}

-- The real Kubota, Knoche and weed unit packages through the shared command path: hitch lift
-- and lower, driving under pressure changes, ball_tip and tool_tip riding the moving tool
-- bodies. Fails when the machines submodule is missing.
local function real_machines(a)
  local pwd = oslo.sys.pwd()
  local tractor = a.tractor or (pwd .. "/machines/usd/kubota_tractor.usdz")
  local implement = a.implement or (pwd .. "/machines/usd/knoche_disc_harrow.usdz")
  local weedunit = a.weedunit or (pwd .. "/machines/usd/weedunit.usdz")
  for _, asset in ipairs({ tractor, implement, weedunit }) do
    assert(oslo.fs.read(asset) ~= nil, "machines submodule asset is missing: " .. asset)
  end
  oslo.env.set("GEARBOX_BENCH_ASSET", tractor)
  oslo.env.set("GEARBOX_BENCH_TRAILER", implement)
  oslo.env.set("GEARBOX_TIP_ASSET", weedunit)
  contract_tests("real machines", { "-p", "gearbox-sim", "--bin", "gearbox", "--",
    "imported_tractor_lifts_and_lowers_a_mounted_implement", "imported_kubota_pressure_benchmark",
    "real_weedunit_tips_follow_their_tool_bodies", "--ignored", "--test-threads=1" },
    { "controller::benchmark::hitching::imported_tractor_lifts_and_lowers_a_mounted_implement",
      "controller::benchmark::imported_kubota_pressure_benchmark",
      "joint_control::conformance::real_weedunit_tips_follow_their_tool_bodies" })
end

make.recipe{
  name = "test-real-machines",
  desc = "real tractor, implement and weed unit through the joint command contract (needs the machines submodule)",
  params = { { "--tractor", desc = "tractor package (default machines/usd/kubota_tractor.usdz)" },
             { "--implement", desc = "implement package (default machines/usd/knoche_disc_harrow.usdz)" },
             { "--weedunit", desc = "weed unit package (default machines/usd/weedunit.usdz)" } },
  run = function(a) real_machines(a) end,
}

make.recipe{
  name = "audit-machine-assets",
  desc = "read-only audit of every machine package under --asset-root (JSON report, nonzero on required-profile errors)",
  params = { { "--asset-root", desc = "directory of USD packages, e.g. machines/usd" },
             { "--require", desc = "declared (default) or v2" } },
  run = function(a)
    local root = a["asset-root"] or a.asset_root
    assert(type(root) == "string" and root ~= "", "--asset-root=DIR is required")
    local require = a.require or "declared"
    assert(type(require) == "string", "--require=declared|v2")
    local out = ("target/robot-contract/audit-%s.json"):format(os.date("%Y%m%d-%H%M%S"))
    sh.cargo("run", "-q", "-p", "gearbox-sim", "--bin", "gearbox", "--", "audit-machines",
             "--asset-root", root, "--require", require, "--out", out)
  end,
}

make.recipe{
  name = "describe-machines",
  desc = "print the compiled robot manifest of one USD file: --file=PATH [--args='--world']",
  params = { { "--file", desc = "USD file" }, { "--args", desc = "--world or --entry PRIM" } },
  run = function(a)
    assert(type(a.file) == "string", "--file=PATH is required")
    local extra = {}
    for word in forwarded_args(a):gmatch("%S+") do extra[#extra + 1] = word end
    sh.cargo("run", "-q", "-p", "gearbox-sim", "--bin", "gearbox", "--", "describe-machines", a.file, table.unpack(extra))
  end,
}

make.recipe{
  name = "write-robot-goldens",
  desc = "rewrite the golden robot manifests and v1 payloads on purpose; review the diff before keeping it",
  run = function()
    oslo.env.set("GEARBOX_WRITE_GOLDEN", "1")
    sh.cargo("test", "-p", "gearbox-sim", "--bin", "gearbox", "write_golden_manifests", "--", "--ignored")
    sh.cargo("test", "-p", "gearbox-api", "--test", "wire_v2", "write_v1_golden_payloads", "--", "--ignored")
  end,
}

local function control_lifecycle_gate()
  contract_tests("control lifecycle (sim)", { "-p", "gearbox-sim", "--bin", "gearbox", "--",
    "joint_control::lifecycle_tests::", "joint_control::tests::c07", "joint_control::tests::c16",
    "joint_control::tests::c17", "joint_control::tests::c18", "physics::commit_tests::c18",
    "physics::bodies::tests::a_rejected" },
    { "joint_control::lifecycle_tests::lifecycle_inventory_passes",
      "joint_control::lifecycle_tests::c21_v2_leases_follow_the_session_and_a_reset_clears_them",
      "joint_control::tests::c16_live_expiry_and_tick_replay_keep_their_clocks",
      "joint_control::tests::c17_a_stale_generation_is_rejected",
      "joint_control::tests::c18_reset_touches_one_robot",
      "physics::commit_tests::c18_a_reset_dof_returns_to_its_load_time_actuator",
      "physics::bodies::tests::a_rejected_machine_body_is_a_fixed_preview" })
  contract_tests("control lifecycle (wire)", { "-p", "gearbox-api", "--test", "wire_v2" },
    { "v1_payloads_keep_their_bytes", "v2_messages_round_trip" })
end

make.recipe{
  name = "test-control-lifecycle",
  desc = "command lifecycle, authority, expiry, generations, reset and the versioned wire (plans/001)",
  run = control_lifecycle_gate,
}

-- Conformance runs (plans/001) write their evidence under target/robot-contract/<run>/.
local function contract_run_dir(kind)
  local dir = ("%s/target/robot-contract/%s-%s"):format(oslo.sys.pwd(), kind, os.date("%Y%m%d-%H%M%S"))
  assert(oslo.run{ "mkdir", "-p", dir }.ok, "cannot create " .. dir)
  return dir
end

local function native_conformance(dir)
  oslo.env.set("GEARBOX_CONFORMANCE_OUT", dir .. "/native")
  contract_tests("control conformance", { "-p", "gearbox-sim", "--bin", "gearbox", "--",
    "joint_control::conformance::", "controller::e03_mapping_test::", "controller::control_frame_test::",
    "joint_control::lifecycle_tests::tf_v2_sites_follow_their_moving_bodies", "physics::joints::tests::a_world" },
    { "joint_control::conformance::native_conformance_arm_schedule",
      "joint_control::conformance::native_acceleration_drive_follows_the_degree_convention",
      "joint_control::conformance::native_conformance_arm_lifecycle",
      "controller::e03_mapping_test::e03_native_mapping_cases",
      "controller::control_frame_test::the_declared_control_frame_sets_the_drive_direction",
      "joint_control::lifecycle_tests::tf_v2_sites_follow_their_moving_bodies" })
  print("control conformance: native evidence in " .. dir .. "/native")
end

make.recipe{
  name = "test-control-conformance",
  desc = "native conformance harnesses: arm schedule, drive units, drive mapping, control frame, TF sites (plans/001)",
  run = function() native_conformance(contract_run_dir("native")) end,
}

-- Isaac Sim through the Nix launcher: each schedule runs in its own headless Kit child under a
-- wall-clock limit, then is compared with the native run. A missing launcher fails the gate.
local ISAAC_ROOT = "/pkg/isaacsim/6.2.0"
local ISAAC_SCHEDULES = { "e02_arm", "e02_lifecycle", "e01_accel", "e03_mapping" }

local function isaac_contract(dir)
  local found = oslo.run{ "sh", "-c", "command -v isaacsim", capture = true }
  assert(found.ok and found.out ~= "", "isaac contract NOT RUN: no isaacsim launcher on PATH")
  local launcher = found.out:gsub("%s+$", "")
  local version = oslo.fs.read(ISAAC_ROOT .. "/VERSION")
  assert(version, "isaac contract NOT RUN: " .. ISAAC_ROOT .. "/VERSION is missing")
  print(("isaac contract: launcher %s, Isaac %s"):format(launcher, (version:gsub("%s+$", ""))))
  local others = oslo.run{ "sh", "-c", "pgrep -fc '[k]it.*isaacsim' || true", capture = true }
  print("isaac contract: Kit processes already running (left alone): " .. (others.out or "?"):gsub("%s+$", ""))
  native_conformance(dir)
  local pwd = oslo.sys.pwd()
  for _, name in ipairs(ISAAC_SCHEDULES) do
    local out = dir .. "/isaac/" .. name
    local schedule = pwd .. "/tests/fixtures/robot_contract/schedules/" .. name .. ".json"
    assert(oslo.run{ "mkdir", "-p", out }.ok)
    local probe = oslo.run{ "sh", "-c", ("PROBE_SCHEDULE=%q PROBE_OUT=%q PROBE_TIMEOUT=240 timeout --kill-after=15 300 "
      .. "%q --no-window --no-ros-env --exec %q > %q 2>&1"):format(schedule, out, launcher,
      pwd .. "/scripts/robot_contract/isaac_probe.py", out .. "/kit.log") }
    local status = oslo.fs.read(out .. "/status.json")
    assert(status, ("isaac contract: Kit ran no probe for %s (startup failure or --exec syntax); see %s/kit.log"):format(name, out))
    assert(probe.ok, ("isaac contract: probe %s did not complete: %s"):format(name, status))
    local compared = oslo.run{ "python3", pwd .. "/scripts/robot_contract/compare.py", "--schedule", schedule,
      "--native", dir .. "/native/" .. name, "--isaac", out, "--out", dir .. "/compare-" .. name .. ".json" }
    assert(compared.ok, "isaac contract: " .. name .. " differs; see " .. dir .. "/compare-" .. name .. ".json")
  end
  local evidence = { "python3", pwd .. "/scripts/robot_contract/evidence.py", "--run", dir, "--launcher", launcher,
    "--isaac-root", ISAAC_ROOT, "--schedules" }
  for _, name in ipairs(ISAAC_SCHEDULES) do evidence[#evidence + 1] = name end
  assert(oslo.run(evidence).ok, "isaac contract: run manifest reports a failure")
  print("isaac contract: E01, E02, lifecycle, E03 and drive units pass; evidence in " .. dir)
end

make.recipe{
  name = "test-isaac-contract",
  desc = "same fixtures and schedules in native Gearbox and headless Isaac Sim, compared (plans/001)",
  run = function() isaac_contract(contract_run_dir("isaac")) end,
}

-- Representative machines staged as schemaVersion 2 overlays under target/; production
-- packages under machines/ are only read. Promotion needs a reviewed report.
local MIGRATION_ASSETS = {
  "clearpath_husky:/husky:/husky/chassis",
  "weedunit:/weedunit:/weedunit/base_link",
  "kubota_tractor:/robot:/robot/chassis",
  "knoche_disc_harrow:/robot:/robot/chassis",
  "perseverance_nasa:/perseverance_nasa:/perseverance_nasa/base_link",
}

local function stage_migration(all)
  local pwd = oslo.sys.pwd()
  sh.cargo("build", "-p", "gearbox-sim", "--bin", "gearbox")
  local dir = contract_run_dir(all and "migration-all" or "migration")
  local cmd = { pwd .. "/scripts/robot_contract/stage_migration.sh", dir, pwd .. "/target/debug/gearbox", pwd .. "/machines/usd" }
  if not all then
    for _, spec in ipairs(MIGRATION_ASSETS) do cmd[#cmd + 1] = spec end
  end
  assert(oslo.run(cmd).ok, "stage-v2-migration: staging failed")
  sh.cargo("run", "-q", "-p", "gearbox-sim", "--bin", "gearbox", "--", "audit-machines",
           "--asset-root", dir .. "/v2", "--require", "v2", "--out", dir .. "/audit-v2.json")
  return dir
end

make.recipe{
  name = "stage-v2-migration",
  desc = "stage metadata-only schemaVersion 2 overlays (representative machines, or every package with --all) and report before/after (plans/001)",
  params = { { "--all", desc = "stage every package under machines/usd" } },
  run = function(a)
    print("stage-v2-migration: staged in " .. stage_migration(a.all ~= nil) .. "; review reports/ before replacing any package")
  end,
}

-- The machines submodule revision whose packages are the last v1 ones: what a rollback
-- restores together with the rollback runtime.
local MACHINES_V1_REVISION = "676be7dabfd2551b243e029cb376e120b01c0c03"

-- Every package of machines/usd at the v1 revision, extracted under `dir`.
local function v1_revisions(dir)
  local pwd = oslo.sys.pwd()
  assert(oslo.run{ "mkdir", "-p", dir }.ok)
  local listed = oslo.run{ "git", "-C", pwd .. "/machines", "ls-tree", "--name-only", MACHINES_V1_REVISION, "usd/", capture = true }
  assert(listed.ok, "machines submodule has no revision " .. MACHINES_V1_REVISION)
  local count = 0
  for path in listed.out:gmatch("[^\n]+") do
    if path:match("%.usdz$") then
      local file = dir .. "/" .. path:match("([^/]+)$")
      local machines = pwd .. "/machines"
      assert(oslo.run{ "sh", "-c", ("set -o pipefail; git -C %q show %q | git -C %q lfs smudge > %q"):format(
        machines, MACHINES_V1_REVISION .. ":" .. path, machines, file) }.ok, "cannot restore " .. path .. " at the v1 revision")
      count = count + 1
    end
  end
  assert(count > 0, "no v1 package at " .. MACHINES_V1_REVISION)
  return count
end

-- The rollback runtime: this commit built with `schema-v1-only` reads legacy packages and
-- rejects every v2 one, so a rollback cannot misread a migrated asset; it loads the
-- matching v1 revisions a rollback restores.
local function rollback_guard()
  local pwd = oslo.sys.pwd()
  local dir = contract_run_dir("rollback")
  local rollback = pwd .. "/target/rollback"
  sh.cargo("build", "-p", "gearbox-sim", "--bin", "gearbox", "--features", "schema-v1-only", "--target-dir", rollback)
  v1_revisions(dir .. "/v1")
  local check = [[
import json, sys
doc = json.load(open(sys.argv[1]))
want = sys.argv[2] == "accept"
machines = doc["machines"]
ok = bool(machines) and all(m["accepted"] == want for m in machines)
if not want:
    ok = ok and all(any(d["code"] == "unknown_schema_version" for d in m["diagnostics"]) for m in machines)
print(("rollback guard: %s %s %s" % (sys.argv[3], "accepted" if want else "rejected", "as expected" if ok else "UNEXPECTEDLY")))
sys.exit(0 if ok else 1)
]]
  local listed = oslo.run{ "sh", "-c", ("ls %q/machines/usd/*.usdz"):format(pwd), capture = true }
  local checked = 0
  for package in listed.out:gmatch("[^\n]+") do
    local name = package:match("([^/]+)%.usdz$")
    for _, case in ipairs({ { dir .. "/v1/" .. name .. ".usdz", "accept" }, { package, "reject" } }) do
      local out = ("%s/rollback-%s-%s.json"):format(dir, name, case[2])
      oslo.run{ "sh", "-c", ("%q describe-machines %q > %q 2>/dev/null || true"):format(rollback .. "/debug/gearbox", case[1], out) }
      assert(oslo.run{ "python3", "-I", "-c", check, out, case[2], case[1] }.ok, "rollback guard failed on " .. case[1])
    end
    checked = checked + 1
  end
  assert(checked > 0, "no production package to check")
  print(("rollback guard: the schema-v1-only build rejects all %d production v2 packages and loads their v1 revisions; %s"):format(checked, dir))
end

make.recipe{
  name = "test-rollback-guard",
  desc = "the schema-v1-only rollback build rejects the production v2 packages and loads their v1 revisions (plans/001)",
  run = rollback_guard,
}

-- Promotion: stage every package, bake each overlay into a copy of its package under
-- target/, check the copy compiles to the staged model, passes the v2 audit and the real
-- scenarios, and only with --apply replace machines/usd (tracked in the machines submodule).
make.recipe{
  name = "promote-v2-migration",
  desc = "bake staged v2 overlays into the machine packages; --apply replaces machines/usd after the checks pass (plans/001)",
  params = { { "--apply", desc = "replace the production packages once every check passes" } },
  run = function(a)
    local pwd = oslo.sys.pwd()
    local staging = stage_migration(true)
    local out = contract_run_dir("promotion")
    assert(oslo.run{ "python3", "-I", pwd .. "/scripts/robot_contract/promote_v2.py", staging, out, pwd .. "/machines/usd" }.ok, "promotion build failed")
    local same = [[
import json, sys
a, b = (json.load(open(p))["machines"] for p in sys.argv[1:3])
for m in a + b:
    m.pop("load_instance", None)
ok = a == b and all(m["accepted"] and m["profile"] == "gearbox_v2" for m in a)
print("promote: %s %s" % (sys.argv[3], "matches its staged v2 model" if ok else "DIFFERS from its staged v2 model"))
sys.exit(0 if ok else 1)
]]
    local listed = oslo.run{ "sh", "-c", ("ls %q/*.usdz"):format(out), capture = true }
    for package in listed.out:gmatch("[^\n]+") do
      local name = package:match("([^/]+)%.usdz$")
      local manifest = out .. "/" .. name .. ".manifest.json"
      oslo.run{ "sh", "-c", ("%q describe-machines %q > %q 2>/dev/null || true"):format(pwd .. "/target/debug/gearbox", package, manifest) }
      assert(oslo.run{ "python3", "-I", "-c", same, manifest, staging .. "/manifests/" .. name .. ".v2.json", name }.ok, "promote: " .. name)
    end
    sh.cargo("run", "-q", "-p", "gearbox-sim", "--bin", "gearbox", "--", "audit-machines",
             "--asset-root", out, "--require", "v2", "--out", out .. "/audit-v2.json")
    real_machines({ tractor = out .. "/kubota_tractor.usdz", implement = out .. "/knoche_disc_harrow.usdz", weedunit = out .. "/weedunit.usdz" })
    if a.apply == nil then
      print("promote-v2-migration: checked in " .. out .. "; run with --apply to replace machines/usd")
      return
    end
    assert(oslo.run{ "sh", "-c", ("cp %q/*.usdz %q/machines/usd/"):format(out, pwd) }.ok, "copy failed")
    print("promote-v2-migration: machines/usd replaced from " .. out)
  end,
}

make.recipe{
  name = "verify-robot-contract",
  desc = "every robot-contract gate in turn; reports which passed and fails if any did not (plans/001)",
  run = function()
    local pwd = oslo.sys.pwd()
    local gates = {
      { "machine contract", machine_contract_gate },
      { "joint control", joint_control_gate },
      { "control lifecycle", control_lifecycle_gate },
      { "hitch and services", function()
        contract_tests("hitch", { "-p", "gearbox-sim", "--bin", "gearbox", "services::" }, {}) end },
      { "isaac contract (with native conformance)", function() isaac_contract(contract_run_dir("verify")) end },
      { "asset audit", function()
        sh.cargo("run", "-q", "-p", "gearbox-sim", "--bin", "gearbox", "--", "audit-machines", "--asset-root",
          pwd .. "/machines/usd", "--require", "declared", "--out", pwd .. "/target/robot-contract/audit-verify.json") end },
      { "real machines (production v2 packages)", function() real_machines({}) end },
      { "real machines (v1 revisions)", function()
        local dir = contract_run_dir("v1")
        v1_revisions(dir)
        real_machines({ tractor = dir .. "/kubota_tractor.usdz", implement = dir .. "/knoche_disc_harrow.usdz",
                        weedunit = dir .. "/weedunit.usdz" }) end },
      { "rollback guard", rollback_guard },
      { "every production package is v2", function()
        sh.cargo("run", "-q", "-p", "gearbox-sim", "--bin", "gearbox", "--", "audit-machines", "--asset-root",
          pwd .. "/machines/usd", "--require", "v2", "--out", pwd .. "/target/robot-contract/audit-v2-production.json") end },
    }
    local failed = {}
    for _, gate in ipairs(gates) do
      local ok, err = pcall(gate[2])
      gate.ok, gate.err = ok, err
      if not ok then failed[#failed + 1] = gate[1] end
    end
    print("\nrobot contract gates:")
    for _, gate in ipairs(gates) do
      print(("  %-42s %s%s"):format(gate[1], gate.ok and "pass" or "FAIL", gate.ok and "" or ("  " .. tostring(gate.err))))
    end
    assert(#failed == 0, "robot contract: failed gates: " .. table.concat(failed, ", "))
  end,
}

make.recipe{ name = "check", desc = "cargo check on all targets",
             run = function() sh.cargo("check", "--all-targets") end }

make.recipe{ name = "fmt", desc = "format the workspace",
             run = function() sh.cargo("fmt", "--all") end }
make.recipe{ name = "fmt-file", desc = "format one Rust source file",
             params = { { "--file", desc = "Rust source path" } },
             run = function(a) assert(a.file, "--file is required"); sh.rustfmt("--edition", "2024", a.file) end }

make.recipe{ name = "clean", desc = "remove Cargo build artifacts",
             run = function() sh.cargo("clean") end }

make.recipe{ name = "bind", desc = "generate both C and Python bindings",
             deps = { "bind-c", "bind-py" } }

make.recipe{
  name = "bind-c",
  desc = "generate the C header",
  run = function()
    sh.cargo("build", "--lib")
    sh.cbindgen("--config", "cbindgen.toml", "--crate", NAME, "--output", "include/" .. NAME .. ".h")
  end,
}

make.recipe{
  name = "bind-py",
  desc = "generate the Python bindings",
  run = function() sh.maturin("build", "--features", "python") end,
}

make.recipe{
  name = "docs",
  desc = "build the mdbook site into docs/, and commit it",
  run = function()
    need("mdbook", "mdbook is not installed; install it first")
    local top = oslo.sys.pwd()
    sh.mdbook("build", top .. "/book", "--dest-dir", top .. "/docs")
    sh.git("add", "--all")
    sh.git("commit", "-m", "docs: building website/mdbook")
  end,
}

make.recipe{
  name = "release",
  desc = "cut a version: --type patch | minor | major | M.m.p",
  params = { { "--type", desc = "patch | minor | major | M.m.p" } },
  run = function(a)
    need("git-rel", "git-rel is not installed; install it first")
    assert(type(a.type) == "string",
           "which release? make release --type patch|minor|major|M.m.p")
    sh.git("rel", a.type)
  end,
}
