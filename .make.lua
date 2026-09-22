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
  params = { { "--args", desc = "passed through to `gearbox run`, e.g. a USD scene path" } },
  deps = { "build" },
  run = function(a)
    sh.sh("-c", fresh_display_env(
      ("env WINIT_UNIX_BACKEND=$_gb_backend nixVulkan target/release/gearbox run %s"):format(a.args or "")))
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
      ("env WINIT_UNIX_BACKEND=$_gb_backend nixVulkan target/release/gearbox launch %s"):format(a.args or "")))
  end,
}

make.recipe{
  name = "cli",
  desc = "run the gearbox CLI: make cli --args 'instance list'",
  params = { { "--args", desc = "arguments passed through to the CLI" } },
  deps = { "build" },
  run = function(a)
    sh.sh("-c", ("target/release/gearbox %s"):format(a.args or ""))
  end,
}

make.recipe{ name = "test", desc = "run all tests",
             run = function() sh.cargo("test", "--all-targets") end }
make.alias("t", "test")
make.recipe{ name = "test-tracked-machine", desc = "test the imported GEARBOX_TRACK_ASSET through native tracked control",
             run = function() sh.cargo("test", "-p", "gearbox-sim", "real_tracked_machine", "--", "--ignored", "--nocapture") end }
make.recipe{ name = "test-fem", desc = "test native FEM scheduling",
             run = function() sh.cargo("test", "-p", "gearbox-sim", "--bin", "gearbox", "physics::fem") end }
make.recipe{ name = "test-fem-gpu", desc = "test native FEM scheduling on a real GPU",
             run = function() sh.nixVulkan("cargo", "test", "-p", "gearbox-sim", "--bin", "gearbox", "physics::fem", "--", "--ignored", "--nocapture", "--test-threads=1") end }

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
