-- gearbox's directory environment. Loaded when you `cd` here, unloaded when you leave.

-- flake.nix reads NVIDIA_VERSION through Nix's impure `builtins.getEnv` to decide, once,
-- at devshell evaluation time, whether to build `nixVulkanNvidia-<version>` and what the
-- `nixVulkan` alias symlinks to. If NVIDIA_VERSION isn't set before nix_develop() runs,
-- that alias silently bakes in nixVulkanIntel instead — permanently, for the whole
-- session, regardless of what real hardware is present. So this has to run first.
--
-- Presence is checked against the PCI bus, not the driver version file directly: that
-- proc entry is tied to the kernel module being loaded at this exact instant and has been
-- observed to briefly read as absent even while `lspci` sees the card fine moments before
-- and after (module reload / on-demand-load race), so this retries a few times before
-- giving up — cheap on a machine with no NVIDIA GPU (every retry misses instantly), but
-- saves a real GPU's driver version from being missed during a momentary reload.
local function detect_nvidia_version()
  local pci = oslo.run{ "sh", "-c", "lspci -d 10de::03xx 2>/dev/null", capture = true }
  if not pci.ok or not pci.out or pci.out:match("^%s*$") then
    return nil
  end
  for _ = 1, 5 do
    local ver = oslo.run{
      "sh", "-c",
      "head -n1 /proc/driver/nvidia/version 2>/dev/null | sed -nE 's/.*  ([0-9.]+)  Release.*/\\1/p'",
      capture = true,
    }
    if ver.ok and ver.out and not ver.out:match("^%s*$") then
      return ver.out:match("^%s*(.-)%s*$")
    end
    oslo.run{ "sh", "-c", "sleep 0.3" }
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
