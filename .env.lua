-- gearbox's directory environment. Loaded when you `cd` here, unloaded when you leave.

oslo.direnv.nix_develop()

oslo.direnv.path_add("./target/debug")
oslo.direnv.path_add("./target/release")

oslo.env.set("TOP_HEAD", oslo.sys.pwd())

-- Wayland/NVIDIA render-offload detection lives entirely in `.make.lua`'s `run`/`sim`
-- recipes (self-contained, no external file): they detect fresh and invoke `nix develop`
-- themselves rather than relying on this file's one-time, cached devshell activation — see
-- `.make.lua` for why (this file's own attempt at it never actually worked: `oslo.env.set`
-- doesn't take effect within the same script, only after it returns, so a `nix_develop()`
-- call here can never see a value this file just tried to set).

oslo.env.set_alias("_b", "make build")
oslo.env.set_alias("_c", "make compile")
oslo.env.set_alias("_r", "make run")
oslo.env.set_alias("_t", "make test")
