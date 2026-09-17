-- gearbox's directory environment. Loaded when you `cd` here, unloaded when you leave.

oslo.direnv.nix_develop()

oslo.direnv.path_add("./target/debug")
oslo.direnv.path_add("./target/release")

oslo.env.set("TOP_HEAD", oslo.sys.pwd())

-- Wayland/NVIDIA render-offload detection lives entirely in `.make.lua`'s `run`/`sim`
-- recipes now (self-contained, no external file) — it runs fresh on every launch rather
-- than once here, so it can't go stale for the rest of the shell session the way sourcing
-- it here once did.

oslo.env.set_alias("_b", "make build")
oslo.env.set_alias("_c", "make compile")
oslo.env.set_alias("_r", "make run")
oslo.env.set_alias("_t", "make test")
