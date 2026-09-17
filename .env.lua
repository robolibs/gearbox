-- gearbox's directory environment. Loaded when you `cd` here, unloaded when you leave.

oslo.direnv.nix_develop()

oslo.direnv.path_add("./target/debug")
oslo.direnv.path_add("./target/release")

oslo.env.set("TOP_HEAD", oslo.sys.pwd())

-- Wayland/NVIDIA render-offload detection lives entirely in `.make.lua`'s `run`/`sim`
-- recipes (self-contained, no external file): they detect fresh and invoke `nix develop`
-- themselves rather than relying on this file's devshell activation for the NVIDIA choice
-- — see `.make.lua` for why (a same-script `oslo.env.set()` here doesn't reach
-- `nix_develop()`'s own subprocess; filed as ISSUE_OSLO_MAKE.md upstream).

oslo.env.set_alias("_b", "make build")
oslo.env.set_alias("_c", "make compile")
oslo.env.set_alias("_r", "make run")
oslo.env.set_alias("_t", "make test")
