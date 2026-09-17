-- gearbox's build, as recipes. This replaced the Makefile; there is no other.
--
--   make            the recipes, with what each of them says it does
--   make build      everything in the workspace (debug)
--   make run        the simulator, through the CLI
--   make test       the suite
--
-- At an oslo prompt in this directory `make` is enough; everywhere else it is `oslo make`.
-- The dev shell's toolchain comes from `.env.lua`'s `nix_develop()`, so recipes call `cargo`
-- directly rather than wrapping every command in `nix develop -c`.

local make = oslo.make

-- `run`/`sim` re-detect the Wayland socket and (real) NVIDIA presence fresh, every
-- launch, instead of trusting whatever BACKEND/DISPLAY a shell happened to inherit — a
-- stale BACKEND=x11 (no live Wayland socket seen the first time this shell started, or a
-- multiplexer/SSH session with a stubborn old DISPLAY) silently falls back to XWayland,
-- which is dramatically slower than the native Wayland + Vulkan path.
--
-- Self-contained on purpose, not sourced from the shared `.display.sh`: that script's
-- NVIDIA check unconditionally exported `__GLX_VENDOR_LIBRARY_NAME=nvidia` and
-- `__VK_LAYER_NV_optimus=NVIDIA_only` regardless of whether NVIDIA hardware was actually
-- present (its detection function always returned success — a bare `if` with no `else`
-- exits 0 whether or not the test matched), and never actually switched RUN_WITH to the
-- Intel/Mesa wrapper on a machine without NVIDIA either. Forcing the NVIDIA vendor on a
-- box with no NVIDIA GPU is exactly the "crazy slow" symptom this must never reproduce.
--
-- Presence is checked against the PCI bus (`lspci`), not `/proc/driver/nvidia/version`:
-- that proc entry is tied to the kernel module being loaded at this exact instant and has
-- been observed to read as briefly absent even while `nvidia-smi`/`lspci` see the card
-- fine moments before and after (module reload / on-demand-load race) — a real NVIDIA
-- laptop hit exactly this and silently fell back to llvmpipe software rendering.
--
-- Even the PCI bus itself was then observed to briefly not list the card right after a
-- `cargo build` finished — PRIME/Optimus runtime power management on some laptops
-- electrically powers the discrete GPU down while it's unused (a CPU-only build never
-- touches it) and it takes a moment to come back once something asks for it. So this
-- retries a few times over ~2s before believing "no NVIDIA" — cheap on a machine that
-- truly has none (every retry misses instantly), but saves a real GPU from being
-- mistaken for absent during its own wake-up.
--
-- Deliberately does NOT use the names `BACKEND`/`RUN_WITH`/`WAYLAND_DISPLAY`/`DISPLAY` for
-- its own computed values: on at least one machine those names came back `readonly` in the
-- interactive shell (set once, presumably by the same `.env.lua`/`.display.sh` activation
-- on `cd`, then locked), so `unset`/`export` on them silently no-ops and this whole
-- detection block would compute the right answer and then have it thrown away. `_gb_*`
-- names can't collide with anything set before this script runs.
local function fresh_display_env(cmd)
  return ([[
set -x
_gb_xdg_runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
# An inherited WAYLAND_DISPLAY can name a socket that doesn't exist any more
# — a compositor restart hands out a fresh randomized socket name, and a
# long-lived shell keeps the old one exported forever. So this never trusts
# the name alone: it only uses it if that exact socket is still live, then
# falls back to the conventional wayland-0, then as a last resort picks
# whichever wayland-* socket in the runtime dir was touched most recently
# (the one a fresh compositor session would have just made).
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
echo "gpu-detect: lspci=$(command -v lspci || echo 'NOT FOUND')" 1>&2
_gb_has_nvidia=0
for _gb_attempt in 1 2 3 4 5; do
  # 10de = NVIDIA's PCI vendor ID (stable, not text-matched — lspci prints raw
  # hex instead of "NVIDIA Corporation" when its pci.ids database is missing
  # or stale, which would silently break a name-based grep). 03xx = any
  # display-class device, not just 0300 "VGA compatible controller": a
  # laptop's discrete GPU with no display output wired to it commonly
  # enumerates as 0302 "3D controller" instead, and 0300-only would never
  # see it regardless of which physical PCI slot/lane it's actually on.
  _gb_lspci_out=$(lspci -d 10de::03xx 2>&1)
  echo "gpu-detect: attempt $_gb_attempt: [$_gb_lspci_out]" 1>&2
  if [ -n "$_gb_lspci_out" ]; then
    _gb_has_nvidia=1
    break
  fi
  sleep 0.4
done
if [ "$_gb_has_nvidia" = 1 ]; then
  export __NV_PRIME_RENDER_OFFLOAD=1
  export __NV_PRIME_RENDER_OFFLOAD_PROVIDER=NVIDIA-G0
  export __GLX_VENDOR_LIBRARY_NAME=nvidia
  export __VK_LAYER_NV_optimus=NVIDIA_only
  _gb_run_with=nixVulkan
else
  unset __NV_PRIME_RENDER_OFFLOAD __NV_PRIME_RENDER_OFFLOAD_PROVIDER __GLX_VENDOR_LIBRARY_NAME __VK_LAYER_NV_optimus
  _gb_run_with=nixVulkanIntel
fi
echo "gpu-detect: resolved backend=$_gb_backend run_with=$_gb_run_with" 1>&2
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
  desc = "launch the simulator through the CLI (fresh Wayland/NVIDIA detection, nixVulkan wrapper)",
  params = { { "--args", desc = "passed through to `gearbox run`, e.g. a USD scene path" } },
  deps = { "build" },
  run = function(a)
    sh.sh("-c", fresh_display_env(
      ("env WINIT_UNIX_BACKEND=$_gb_backend $_gb_run_with target/debug/gearbox run %s"):format(a.args or "")))
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
      ("env WINIT_UNIX_BACKEND=$_gb_backend $_gb_run_with target/debug/gearbox launch %s"):format(a.args or "")))
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
