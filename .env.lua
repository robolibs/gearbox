-- gearbox's directory environment. Loaded when you `cd` here, unloaded when you leave.

-- flake.nix reads NVIDIA_VERSION through Nix's impure `builtins.getEnv` to decide, once,
-- at devshell evaluation time, whether to build `nixVulkanNvidia-<version>` and what the
-- `nixVulkan` alias symlinks to. If NVIDIA_VERSION isn't set before nix_develop() runs,
-- that alias silently bakes in nixVulkanIntel instead — permanently, for the whole
-- session, regardless of what real hardware is present. So this has to run first.
--
-- Pure Lua/`oslo.fs`, no shelling out to `lspci`/`sh` at all: this runs as part of an
-- interactive login shell's own directory-entry hook (this machine's shell IS `oslo`), a
-- context that can have a much more minimal PATH than a one-off `oslo make` invocation
-- does — `lspci` silently not being found there was exactly what caused this to keep
-- computing "no NVIDIA" even right after being fixed and re-verified working from a
-- plain shell. `/sys/bus/pci/devices/*/vendor` is the kernel's own PCI bus enumeration,
-- always readable with no external command, and reflects hardware presence regardless of
-- whether the nvidia kernel module happens to be loaded at this exact instant.
--
-- The driver *version* (for the `nixVulkanNvidia-<version>` package name) does need the
-- module loaded, via /proc/driver/nvidia/version, which can briefly lag behind hardware
-- presence (module reload / on-demand-load race) — so only that read retries, with a
-- pure-Lua busy-wait (no external `sleep` either) between attempts.
local function busy_wait(seconds)
  local start = os.clock()
  while os.clock() - start < seconds do end
end

local function detect_nvidia_version()
  local has_nvidia = false
  for _, path in ipairs(oslo.fs.glob("/sys/bus/pci/devices/*/vendor")) do
    local vendor = oslo.fs.read(path)
    if vendor and vendor:match("^%s*0x10de%s*$") then
      has_nvidia = true
      break
    end
  end
  if not has_nvidia then
    return nil
  end
  for _ = 1, 5 do
    local ver = oslo.fs.read("/proc/driver/nvidia/version")
    if ver then
      local version = ver:match("%s%s(%d[%d%.]*)%s%sRelease")
      if version then
        return version
      end
    end
    busy_wait(0.3)
  end
  return nil
end

local nvidia_version = detect_nvidia_version()
if nvidia_version then
  oslo.env.set("NVIDIA_VERSION", nvidia_version)
  -- PRIME render-offload: routes rendering through the discrete GPU on a hybrid laptop
  -- where the compositor itself runs on the integrated one. Session-wide is correct here
  -- (this doesn't change mid-session the way the Wayland socket can), which is also why
  -- `.make.lua` no longer sets these itself.
  oslo.env.set("__NV_PRIME_RENDER_OFFLOAD", "1")
  oslo.env.set("__NV_PRIME_RENDER_OFFLOAD_PROVIDER", "NVIDIA-G0")
  oslo.env.set("__GLX_VENDOR_LIBRARY_NAME", "nvidia")
  oslo.env.set("__VK_LAYER_NV_optimus", "NVIDIA_only")
end

oslo.direnv.nix_develop()

oslo.direnv.path_add("./target/debug")
oslo.direnv.path_add("./target/release")

oslo.env.set("TOP_HEAD", oslo.sys.pwd())

-- `make run`/`make sim` still do their own fresh Wayland-socket detection at launch time
-- (see `.make.lua`) — that part genuinely can change mid-session (compositor restart).
-- The NVIDIA/`nixVulkan` choice above is session-wide and doesn't need re-checking per
-- launch, since it's what the flake's devshell was actually built with.

oslo.env.set_alias("_b", "make build")
oslo.env.set_alias("_c", "make compile")
oslo.env.set_alias("_r", "make run")
oslo.env.set_alias("_t", "make test")
