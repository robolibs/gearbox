# Vegetation sources

Plant models from [Poly Haven](https://polyhaven.com/models), CC0 1.0 (public domain):
`grass_bermuda_01`, `grass_medium_01`, `grass_medium_02`, `shrub_sorrel_01`,
`celandine_01`, `dandelion_01`, `weed_plant_02`, `nettle_plant`. Each folder holds the
1k glTF, its textures and the separate 1k alpha map (`*_alpha_1k.png`).

`scripts/pack_vegetation.py` picks clumps out of these into
`crates/gearbox-fields/src/clumps/` (`meshes/*.bin`, `textures/*.png`), which the field
profiles draw as instanced layers. Edit the `PACKS` table there and rerun it from the
repository root to change which clumps are used.
