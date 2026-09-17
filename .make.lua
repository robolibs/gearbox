-- gearbox's build, as recipes. This replaced the Makefile; there is no other.
--
--   make            the recipes, with what each of them says it does
--   make build      everything in the workspace (debug)
--   make run        the simulator, through the CLI
--   make test       the suite
--
-- At an oslo prompt in this directory `make` is enough; everywhere else it is `oslo make`.
-- The dev shell's toolchain comes from `.env.lua`'s `nix_develop()`, so most recipes call
-- `cargo` directly rather than wrapping every command in `nix develop -c`. `run`/`sim` are
-- the exception — they invoke `nix develop -c` themselves; see `fresh_display_env` below
-- for why.

local make = oslo.make

-- `run`/`sim` re-detect the Wayland socket and NVIDIA presence fresh, every launch, and
-- invoke `nix develop` themselves — deliberately NOT relying on `.env.lua`'s one-time,
-- already-cached devshell activation (or oslo's own `oslo.env.set`) for the NVIDIA choice.
--
-- gearbox's flake.nix decides once, at devshell-evaluation time, whether `nixVulkan`
-- symlinks to `nixVulkanNvidia-<version>` or `nixVulkanIntel`, by reading NVIDIA_VERSION
-- through Nix's impure `builtins.getEnv`. Getting a correctly-detected value in front of
-- that read turned out to need real care:
--   - `oslo.env.set(...)` in `.env.lua` does NOT set a real process environment variable
--     immediately — it only queues something for the *calling* shell to pick up after the
--     whole script returns, so a `nix_develop()` call later in that same script can never
--     see it (proven: an unconditional `error()` at the top of `.env.lua` doesn't even
--     fire during `oslo make <recipe>` — that file isn't evaluated by these commands at
--     all, only by the interactive shell's own directory-entry hook).
--   - Nix's evaluation sandbox blocks raw file reads like `/proc/driver/nvidia/version`
--     even with `--impure` (`builtins.readFile` "succeeds" but returns empty), so
--     `flake.nix` reading the version itself isn't an option either.
--   - The one thing that reliably works: a real shell `export FOO=...` followed by
--     `nix develop --impure -c ...` *in that same process* — env vars genuinely are
--     inherited through the sandbox for `builtins.getEnv`, just not files. So this runs
--     `nix develop` itself, right after exporting, instead of trusting whatever devshell
--     was already active.
--
-- NVIDIA presence is checked via /sys/bus/pci/devices/*/vendor (the kernel's own PCI bus
-- enumeration — no `lspci` dependency, so no PATH-availability risk), which reflects
-- hardware presence regardless of whether the nvidia kernel module is loaded at this
-- instant. The driver *version* does need the module loaded, via
-- /proc/driver/nvidia/version, which can briefly lag behind hardware presence (module
-- reload / on-demand-load race), so only that read retries.
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
unset NVIDIA_VERSION
for _gb_vendor_file in /sys/bus/pci/devices/*/vendor; do
  if [ "$(cat "$_gb_vendor_file" 2>/dev/null)" = "0x10de" ]; then
    for _gb_attempt in 1 2 3 4 5; do
      _gb_ver=$(sed -nE 's/.*  ([0-9.]+)  Release.*/\1/p' /proc/driver/nvidia/version 2>/dev/null | head -n1)
      if [ -n "$_gb_ver" ]; then
        export NVIDIA_VERSION="$_gb_ver"
        break
      fi
      sleep 0.4
    done
    break
  fi
done
exec nix develop --impure -c %s
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

make.recipe{
  name = "build",
  desc = "everything in the workspace (debug) — every crate, lib and bin",
  run = function() sh.cargo("build") end,
}
make.alias("b", "build")

make.recipe{
  name = "build-release-bin",
  desc = "the merged gearbox binary (release)",
  run = function()
    sh.cargo("build", "--release", "-p", "gearbox-sim", "--bin", "gearbox")
  end,
}

-- Debug sim with per-system Chrome tracing; run it with GEARBOX_TRACE=file.json.
make.recipe{
  name = "build-profile",
  desc = "debug binary built with the profile feature (GEARBOX_TRACE=file.json to use it)",
  run = function()
    sh.cargo("build", "-p", "gearbox-sim", "--bin", "gearbox",
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
      ("env WINIT_UNIX_BACKEND=$_gb_backend nixVulkan target/debug/gearbox run %s"):format(a.args or "")))
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
      ("env WINIT_UNIX_BACKEND=$_gb_backend nixVulkan target/debug/gearbox launch %s"):format(a.args or "")))
  end,
}

make.recipe{
  name = "cli",
  desc = "run the gearbox CLI: make cli --args 'instance list'",
  params = { { "--args", desc = "arguments passed through to the CLI" } },
  deps = { "build" },
  run = function(a)
    sh.sh("-c", ("target/debug/gearbox %s"):format(a.args or ""))
  end,
}

make.recipe{ name = "test", desc = "run all tests",
             run = function() sh.cargo("test", "--all-targets") end }
make.alias("t", "test")

make.recipe{ name = "check", desc = "cargo check on all targets",
             run = function() sh.cargo("check", "--all-targets") end }

make.recipe{ name = "fmt", desc = "format the workspace",
             run = function() sh.cargo("fmt", "--all") end }

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
