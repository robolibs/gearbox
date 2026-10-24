# Crop leaf atlases

Each atlas is a row of four leaves cut from a photographed leaf set, tip up,
the leaf's base at the bottom centre of its cell. The greenhouse's crop
prefabs (`bin/gearbox/src/greenhouse/crops.rs`) bend, cup and arrange them
into plants the way Wageningen's LettuceVisSim builds lettuce from real leaf
images.

| Atlas | Leaves | Source | Licence |
|---|---|---|---|
| `lettuce_leaves.png` | LeafSet009, leaves 2, 3, 0, 1, flipped at build time so the broad end lies outward | [ambientCG LeafSet009](https://ambientcg.com/view?id=LeafSet009) | CC0 |
| `tomato_leaves.png` | LeafSet014, leaves 0, 1, 2, 4 | [ambientCG LeafSet014](https://ambientcg.com/view?id=LeafSet014) | CC0 |
| `potato_leaves.png` | LeafSet024, leaves 0, 1, 2, 4 | [ambientCG LeafSet024](https://ambientcg.com/view?id=LeafSet024) | CC0 |
| `wheat_leaves.png` | LeafSet022, leaves 1, 2, 4, 5, drawn out into blades | [ambientCG LeafSet022](https://ambientcg.com/view?id=LeafSet022) | CC0 |

No CC0 scans of whole crop plants were found (Poly Haven has none, and the
lettuce models in Objaverse are harvested heads or salads), so the plants are
built from leaves rather than taken whole.

Rebuild with `scripts/crops/pack_leaves.py` (its docstring says where the
leaf sets go). To look at the prefabs in a modelling tool, export them:
`cargo test -p gearbox-sim --bin gearbox export_crop_prefabs -- --ignored`
writes OBJ files and the atlases to `target/crops/prefabs/`.
