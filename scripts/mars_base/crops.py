"""What each crop needs from a tray: how far apart its plants (or clumps)
stand, the light it is grown under (BVAD Table 4-96: its daily light over
its photoperiod), and how long a tray of it stays in use (harvest age, plus
bearing for fruiting crops) so trays sown in turn cover every age. Names are
gearbox-greenhouse's crop names."""

# Metres between plants, or clumps for crops a place holds many of.
PITCH = {
    "lettuce": 0.22, "spinach": 0.18, "chard": 0.28, "kale": 0.28, "pak_choi": 0.2, "green_onion": 0.15,
    "radish": 0.18, "carrot": 0.18, "red_beet": 0.2,
    "tomato": 0.4, "pepper": 0.33, "strawberry": 0.3, "snap_bean": 0.28, "pea": 0.2,
    "soybean": 0.2, "dry_bean": 0.36, "peanut": 0.36, "potato": 0.38, "sweet_potato": 0.28,
    "wheat": 0.16, "rice": 0.16,
}

# (µmol/m²/s, hours a day). Wheat is lit at half its 115 mol a day: the
# BPC grew it lower than Bugbee's maximum, and so must a farm on fission power.
LIGHT = {
    "lettuce": (295, 16), "spinach": (295, 16), "chard": (295, 16), "kale": (295, 16), "pak_choi": (295, 16), "green_onion": (450, 16),
    "radish": (450, 16), "carrot": (486, 16), "red_beet": (295, 16),
    "tomato": (625, 12), "pepper": (625, 12), "strawberry": (509, 12), "snap_bean": (370, 18), "pea": (417, 16),
    "soybean": (648, 12), "dry_bean": (556, 12), "peanut": (625, 12), "potato": (648, 12), "sweet_potato": (432, 18),
    "wheat": (800, 20), "rice": (764, 12),
}

# Days a tray of the crop is in use: harvest age, and bearing for fruit.
CYCLE = {
    "lettuce": 30, "spinach": 30, "chard": 45, "kale": 50, "pak_choi": 35, "green_onion": 50,
    "radish": 25, "carrot": 75, "red_beet": 40,
    "tomato": 170, "pepper": 145, "strawberry": 220, "snap_bean": 105, "pea": 75,
    "soybean": 86, "dry_bean": 63, "peanut": 110, "potato": 105, "sweet_potato": 120,
    "wheat": 62, "rice": 88,
}


def slots(species, size, rows=None, pitch=None):
    """Places across a tray of `size` (x, y) for `species`: (cols, rows), the
    rows matched to `rows` gutters when the tray has them; `pitch` overrides
    the crop's own spacing, as for seedlings in a nursery's plug trays."""
    pitch = pitch or PITCH[species]
    cols = max(1, round(size[0] / pitch))
    if rows is None:
        return cols, max(1, round(size[1] / pitch))
    # Every gutter, or every second one for crops wider than a gutter's pitch.
    gutter = size[1] / rows
    return cols, rows if pitch <= gutter * 1.3 else max(1, rows // 2)
