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

-- Vulkan comes from the system driver on NixOS (/run/opengl-driver), elsewhere from nixGL.
local SYSTEM_VULKAN = #oslo.fs.glob("/run/opengl-driver/lib/*") > 0
local VULKAN = SYSTEM_VULKAN and "" or "nixVulkan "

local function gpu(cmd, ...)
  if SYSTEM_VULKAN then return sh[cmd](...) end
  return sh.nixVulkan(cmd, ...)
end

make.recipe{
  name = "run",
  desc = "launch the simulator through the CLI (fresh Wayland detection; nixVulkan off NixOS)",
  params = { { "--args", desc = "gearbox run arguments; use --args='--planet mars ...'" } },
  deps = { "build" },
  run = function(a)
    sh.sh("-c", fresh_display_env(
      ("env WINIT_UNIX_BACKEND=$_gb_backend %starget/release/gearbox run %s"):format(VULKAN, forwarded_args(a))))
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
      ("env WINIT_UNIX_BACKEND=$_gb_backend %starget/release/gearbox launch %s"):format(VULKAN, forwarded_args(a))))
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
             run = function() gpu("cargo", "test", "-p", "gearbox-planet", "--test", "regolith_gpu", "--", "--ignored", "--nocapture") end }
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
             run = function(a) gpu("cargo", "test", "-p", "gearbox-sim", "--bin", "gearbox", a.filter or "physics::fem", "--", "--ignored", "--nocapture", "--test-threads=1") end }

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
  desc = "read-only audit of every machine package under --asset-root (JSON report, nonzero when any machine is rejected)",
  params = { { "--asset-root", desc = "directory of USD packages, e.g. machines/usd" },
             { "--out", desc = "report path (default target/robot-contract/audit-<time>.json)" } },
  run = function(a)
    local root = a["asset-root"] or a.asset_root
    assert(type(root) == "string" and root ~= "", "--asset-root=DIR is required")
    local out = a.out or ("target/robot-contract/audit-%s.json"):format(os.date("%Y%m%d-%H%M%S"))
    sh.cargo("run", "-q", "-p", "gearbox-sim", "--bin", "gearbox", "--", "audit-machines",
             "--asset-root", root, "--out", out)
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
  desc = "rewrite the golden robot manifests on purpose; review the diff before keeping it",
  run = function()
    oslo.env.set("GEARBOX_WRITE_GOLDEN", "1")
    sh.cargo("test", "-p", "gearbox-sim", "--bin", "gearbox", "write_golden_manifests", "--", "--ignored")
  end,
}

local function control_lifecycle_gate()
  contract_tests("control lifecycle (sim)", { "-p", "gearbox-sim", "--bin", "gearbox", "--",
    "joint_control::lifecycle_tests::", "joint_control::tests::c07", "joint_control::tests::c16",
    "joint_control::tests::c17", "joint_control::tests::c18", "physics::commit_tests::c18",
    "physics::bodies::tests::a_rejected" },
    { "joint_control::lifecycle_tests::lifecycle_inventory_passes",
      "joint_control::lifecycle_tests::c21_leases_follow_the_session_and_a_reset_clears_them",
      "joint_control::tests::c16_live_expiry_and_tick_replay_keep_their_clocks",
      "joint_control::tests::c17_a_stale_generation_is_rejected",
      "joint_control::tests::c18_reset_touches_one_robot",
      "physics::commit_tests::c18_a_reset_dof_returns_to_its_load_time_actuator",
      "physics::bodies::tests::a_rejected_machine_body_is_a_fixed_preview" })
  contract_tests("control lifecycle (wire)", { "-p", "gearbox-api", "--test", "wire_control" },
    { "control_messages_round_trip" })
end

make.recipe{
  name = "test-control-lifecycle",
  desc = "command lifecycle, authority, expiry, generations, reset and the control wire messages (plans/001)",
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
    "joint_control::lifecycle_tests::tf_sites_follow_their_moving_bodies", "physics::joints::tests::a_world" },
    { "joint_control::conformance::native_conformance_arm_schedule",
      "joint_control::conformance::native_acceleration_drive_follows_the_degree_convention",
      "joint_control::conformance::native_conformance_arm_lifecycle",
      "controller::e03_mapping_test::e03_native_mapping_cases",
      "controller::control_frame_test::the_declared_control_frame_sets_the_drive_direction",
      "joint_control::lifecycle_tests::tf_sites_follow_their_moving_bodies" })
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
    -- One Kit child per attempt; its exit status lands beside its log. Only a Kit that
    -- never started the probe is tried again, once, and the first attempt's log is kept.
    local function launch(log)
      return oslo.run{ "sh", "-c", ("PROBE_SCHEDULE=%q PROBE_OUT=%q PROBE_TIMEOUT=240 timeout --kill-after=15 300 "
        .. "%q --no-window --no-ros-env --exec %q > %q 2>&1; echo $? > %q.exit"):format(schedule, out, launcher,
        pwd .. "/scripts/robot_contract/isaac_probe.py", log, log) }
    end
    launch(out .. "/kit.log")
    if not oslo.fs.read(out .. "/status.json") then
      local exit = (oslo.fs.read(out .. "/kit.log.exit") or "?"):gsub("%s+$", "")
      assert(oslo.run{ "mv", out .. "/kit.log", out .. "/kit-startup-failure.log" }.ok)
      assert(oslo.run{ "mv", out .. "/kit.log.exit", out .. "/kit-startup-failure.log.exit" }.ok)
      print(("isaac contract: Kit exited %s before running the probe for %s; trying once more (log kept as kit-startup-failure.log)"):format(exit, name))
      launch(out .. "/kit.log")
    end
    local exit = (oslo.fs.read(out .. "/kit.log.exit") or "?"):gsub("%s+$", "")
    local probe = { ok = exit == "0" }
    local status = oslo.fs.read(out .. "/status.json")
    assert(status, ("isaac contract: Kit exited %s and ran no probe for %s (startup failure or --exec syntax); see %s/kit.log"):format(exit, name, out))
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
          pwd .. "/machines/usd", "--out", pwd .. "/target/robot-contract/audit-verify.json") end },
      { "real machines", function() real_machines({}) end },
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

-- Machine packages to and from Blender (machines/scripts/gearbox_usd), in the Blender flatpak.
local function blender(log, blend, ...)
  local pwd = oslo.sys.pwd()
  local words = { "flatpak", "run", "--filesystem=" .. pwd, "--talk-name=org.freedesktop.Flatpak", "org.blender.Blender", "--background" }
  words[#words + 1] = blend or "--factory-startup"
  for _, w in ipairs({ "--python", pwd .. "/machines/scripts/gearbox_usd/cli.py", "--", ... }) do words[#words + 1] = w end
  local quoted = {}
  for i, w in ipairs(words) do quoted[i] = ("%q"):format(w) end
  local r = oslo.run{ "sh", "-c", table.concat(quoted, " ") .. (" > %q 2>&1"):format(log) }
  for line in (oslo.fs.read(log) or ""):gmatch("[^\n]+") do
    if line:find("gearbox_usd:", 1, true) or line:find("Error", 1, true) then print(line) end
  end
  return r.ok
end

local function machine_names(a)
  if a.machine then return { a.machine } end
  local names = {}
  local listed = oslo.run{ "sh", "-c", "ls machines/usd/*.usdz", capture = true }
  for path in (listed.out or ""):gmatch("[^\n]+") do names[#names + 1] = path:match("([^/]+)%.usdz$") end
  return names
end

-- Same compiled machines in two packages, by `gearbox describe-machines`.
local function same_model(a, b, dir)
  local described = {}
  for i, file in ipairs({ a, b }) do
    described[i] = ("%s/%s.%d.json"):format(dir, file:match("([^/]+)%.usdz$"), i)
    oslo.run{ "sh", "-c", ("target/debug/gearbox describe-machines %q > %q 2>/dev/null || true"):format(file, described[i]) }
  end
  local check = [[
import json, sys
a, b = (json.load(open(p))["machines"] for p in sys.argv[1:3])
for m in a + b:
    m.pop("load_instance", None)
sys.exit(0 if a == b and a else 1)
]]
  return oslo.run{ "python3", "-I", "-c", check, described[1], described[2] }.ok
end

make.recipe{
  name = "blender-import",
  desc = "load a machine package into a .blend: --package=PATH [--blend=PATH, default machines/blender/<name>.blend]",
  params = { { "--package", desc = "machine package (.usdz)" }, { "--blend", desc = ".blend to write" } },
  run = function(a)
    assert(type(a.package) == "string", "--package=PATH is required")
    local blend = a.blend or ("machines/blender/" .. a.package:match("([^/]+)%.usdz$") .. ".blend")
    assert(oslo.run{ "mkdir", "-p", "target/blender" }.ok)
    assert(blender("target/blender/import.log", nil, "import", a.package, blend), "blender-import failed; see target/blender/import.log")
  end,
}

make.recipe{
  name = "blender-export",
  desc = "write a .blend's machine as a package: --blend=PATH [--package=PATH, default machines/usd/<name>.usdz]",
  params = { { "--blend", desc = ".blend to export" }, { "--package", desc = "machine package (.usdz) to write" } },
  run = function(a)
    assert(type(a.blend) == "string", "--blend=PATH is required")
    local out = a.package or ("machines/usd/" .. a.blend:match("([^/]+)%.blend$") .. ".usdz")
    assert(oslo.run{ "mkdir", "-p", "target/blender" }.ok)
    assert(blender("target/blender/export.log", a.blend, "export", out), "blender-export failed; see target/blender/export.log")
  end,
}

make.recipe{
  name = "test-blender-roundtrip",
  desc = "every package (or --machine=NAME) through Blender and back: same specs, files and compiled machines",
  params = { { "--machine", desc = "one package name, e.g. kubota_tractor" } },
  run = function(a)
    sh.cargo("build", "-p", "gearbox-sim", "--bin", "gearbox")
    local dir = oslo.sys.pwd() .. "/target/blender/roundtrip"
    assert(oslo.run{ "mkdir", "-p", dir }.ok)
    local failed = {}
    for _, name in ipairs(machine_names(a)) do
      local package = oslo.sys.pwd() .. "/machines/usd/" .. name .. ".usdz"
      local ok = blender(dir .. "/" .. name .. ".log", nil, "roundtrip", package, dir)
      ok = ok and same_model(package, dir .. "/" .. name .. ".usdz", dir)
      print(("blender roundtrip: %-26s %s"):format(name, ok and "pass" or "FAIL"))
      if not ok then failed[#failed + 1] = name end
    end
    local husky = oslo.sys.pwd() .. "/machines/usd/clearpath_husky.usdz"
    local edited = dir .. "/clearpath_husky.edited.usdz"
    local accepted = [[
import json, sys
sys.exit(0 if all(m["accepted"] for m in json.load(open(sys.argv[1]))["machines"]) else 1)
]]
    local ok = blender(dir .. "/edits.log", nil, "edits", husky, edited)
    oslo.run{ "sh", "-c", ("target/debug/gearbox describe-machines %q > %q 2>/dev/null || true"):format(edited, edited .. ".json") }
    ok = ok and oslo.run{ "python3", "-I", "-c", accepted, edited .. ".json" }.ok
    print(("blender roundtrip: %-26s %s"):format("edits in Blender", ok and "pass" or "FAIL"))
    if not ok then failed[#failed + 1] = "edits" end
    sh.cargo("build", "-p", "gearbox-sim", "--bin", "gearbox")
    local addon = blender(dir .. "/addon.log", nil, "addon-check", oslo.sys.pwd() .. "/machines/blender/clearpath_husky.blend", dir .. "/clearpath_husky.addon.usdz")
    print(("blender roundtrip: %-26s %s"):format("add-on with Gearbox check", addon and "pass" or "FAIL"))
    if not addon then failed[#failed + 1] = "add-on" end
    assert(#failed == 0, "blender roundtrip failed: " .. table.concat(failed, ", "))
  end,
}

make.recipe{
  name = "test-blender-export",
  desc = "export every machines/blender .blend (or --machine=NAME) and check it equals its committed package",
  params = { { "--machine", desc = "one machine name, e.g. kubota_tractor" } },
  run = function(a)
    sh.cargo("build", "-p", "gearbox-sim", "--bin", "gearbox")
    local pwd = oslo.sys.pwd()
    local dir = pwd .. "/target/blender/export"
    assert(oslo.run{ "mkdir", "-p", dir }.ok)
    local failed = {}
    for _, name in ipairs(machine_names(a)) do
      local blend, package, out = pwd .. "/machines/blender/" .. name .. ".blend", pwd .. "/machines/usd/" .. name .. ".usdz", dir .. "/" .. name .. ".usdz"
      local ok = oslo.fs.read(blend) ~= nil and blender(dir .. "/" .. name .. ".log", blend, "export", out)
      ok = ok and blender(dir .. "/" .. name .. ".compare.log", nil, "compare", package, out) and same_model(package, out, dir)
      print(("blender export: %-26s %s"):format(name, ok and "pass" or "FAIL"))
      if not ok then failed[#failed + 1] = name end
    end
    assert(#failed == 0, "blender export differs from the package: " .. table.concat(failed, ", "))
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
