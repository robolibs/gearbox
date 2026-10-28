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
| `wheat_leaves.png` | LeafSet022, leaves 1, 2, 4, 5, drawn out into blades; rice and green onion use it too | [ambientCG LeafSet022](https://ambientcg.com/view?id=LeafSet022) | CC0 |
| `spinach_leaves.png` | LeafSet018, leaves 1, 2, 0, 1; spinach, chard, pak choi and red beet | [ambientCG LeafSet018](https://ambientcg.com/view?id=LeafSet018) | CC0 |
| `kale_leaves.png` | LeafSet026, leaves 0, 1, 2, 5, turned tip up | [ambientCG LeafSet026](https://ambientcg.com/view?id=LeafSet026) | CC0 |
| `radish_leaves.png` | LeafSet020, leaves 0 to 3 | [ambientCG LeafSet020](https://ambientcg.com/view?id=LeafSet020) | CC0 |
| `carrot_leaves.png` | LeafSet019, sprigs 1, 2, 0, 1, turned tip up: a conifer sprig stands in for the finely cut frond | [ambientCG LeafSet019](https://ambientcg.com/view?id=LeafSet019) | CC0 |
| `pepper_leaves.png` | LeafSet003, leaves 1, 0, 2, 3 | [ambientCG LeafSet003](https://ambientcg.com/view?id=LeafSet003) | CC0 |
| `strawberry_leaves.png` | LeafSet001, leaves 0, 2, 4, 5 | [ambientCG LeafSet001](https://ambientcg.com/view?id=LeafSet001) | CC0 |
| `bean_leaves.png` | LeafSet023, leaves 0 to 3, turned tip up; soybean, beans, pea and peanut | [ambientCG LeafSet023](https://ambientcg.com/view?id=LeafSet023) | CC0 |
| `sweet_potato_leaves.png` | LeafSet004, leaves 0, 1, 2, 5 | [ambientCG LeafSet004](https://ambientcg.com/view?id=LeafSet004) | CC0 |

No CC0 scans of whole crop plants were found (Poly Haven has none, and the
lettuce models in Objaverse are harvested heads or salads), so the plants are
built from leaves rather than taken whole.

Rebuild with `scripts/crops/pack_leaves.py` (its docstring says where the
leaf sets go). To look at the prefabs in a modelling tool, export them:
`cargo test -p gearbox-sim --bin gearbox export_crop_prefabs -- --ignored`
writes OBJ files and the atlases to `target/crops/prefabs/`.
