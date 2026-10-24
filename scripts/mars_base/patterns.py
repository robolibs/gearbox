"""Procedural texture maps for the base, as numpy arrays: linear colour in
[0, 1], tangent-space normals encoded to [0, 1], roughness in [0, 1].

Each pattern covers one texture tile; builders map a tile to a known size
in metres, so seams and cells land at real spacings.
"""
import numpy as np

N = 1024


def grid(n=N):
    v = (np.arange(n, dtype=np.float32) + 0.5) / n
    return np.meshgrid(v, v)


def noise(n=N, cells=32, seed=0, octaves=4):
    """Tileable value noise in [0, 1]."""
    rng = np.random.default_rng(seed)
    out = np.zeros((n, n), dtype=np.float32)
    amp, total = 1.0, 0.0
    for o in range(octaves):
        c = cells * 2 ** o
        lattice = rng.random((c, c), dtype=np.float32)
        u, v = grid(n)
        x, y = u * c, v * c
        x0, y0 = np.floor(x).astype(int) % c, np.floor(y).astype(int) % c
        x1, y1 = (x0 + 1) % c, (y0 + 1) % c
        fx, fy = x - np.floor(x), y - np.floor(y)
        fx, fy = fx * fx * (3 - 2 * fx), fy * fy * (3 - 2 * fy)
        a = lattice[y0, x0] * (1 - fx) + lattice[y0, x1] * fx
        b = lattice[y1, x0] * (1 - fx) + lattice[y1, x1] * fx
        out += amp * (a * (1 - fy) + b * fy)
        total += amp
        amp *= 0.5
    return out / total


def normals_from_height(height, strength):
    """Tangent-space normals of a tileable height map, encoded to [0, 1]."""
    dx = (np.roll(height, -1, axis=1) - np.roll(height, 1, axis=1)) * strength
    dy = (np.roll(height, -1, axis=0) - np.roll(height, 1, axis=0)) * strength
    n = np.stack([-dx, dy, np.ones_like(height)], axis=-1)
    n /= np.linalg.norm(n, axis=-1, keepdims=True)
    return n * 0.5 + 0.5


def seam(distance, width):
    """1 in a seam's groove, falling to 0 at `width` from its line."""
    return np.clip(1.0 - distance / width, 0.0, 1.0) ** 1.5


def panels(colour, cols=2, rows=4, seam_w=0.004, rivets=True, grime=0.12, seed=1):
    """Painted hull panels: staggered plates, recessed seams, rivet rows and
    light grime. One tile spans `cols` × `rows` plates."""
    u, v = grid()
    row = np.floor(v * rows)
    shift = (row % 2) * 0.5 / cols
    pu = (u + shift) * cols % 1.0
    pv = v * rows % 1.0
    du = np.minimum(pu, 1.0 - pu) / cols
    dv = np.minimum(pv, 1.0 - pv) / rows
    groove = np.maximum(seam(du, seam_w), seam(dv, seam_w))
    height = -groove
    if rivets:
        # Rivets a little in from each plate's edge, every 1/24 of a tile.
        ru = (u * 24.0) % 1.0
        rv = (v * 24.0) % 1.0
        near_v = np.abs(dv - 0.012) < 0.004
        near_u = np.abs(du - 0.012) < 0.004
        dot = np.maximum(near_v * (np.hypot(ru - 0.5, (dv - 0.012) / 0.004 * 0.15) < 0.18),
                         near_u * (np.hypot(rv - 0.5, (du - 0.012) / 0.004 * 0.15) < 0.18))
        height = height + 0.5 * dot
    dirt = noise(cells=8, seed=seed)
    plate = noise(cells=4, seed=seed + 7)[(row.astype(int) * 37) % N, (np.floor((u + shift) * cols).astype(int) * 91) % N]
    shade = 1.0 - grime * dirt - 0.05 * plate - 0.55 * groove
    diff = np.clip(np.array(colour, dtype=np.float32) * shade[..., None], 0.0, 1.0)
    rough = np.clip(0.42 + 0.25 * dirt + 0.3 * groove, 0.0, 1.0)
    return diff, normals_from_height(height, 6.0), rough


def solar(cells_u=6, cells_v=12):
    """A photovoltaic panel: dark blue cells, silver gaps and busbars, an aluminium frame."""
    u, v = grid()
    cu = u * cells_u % 1.0
    cv = v * cells_v % 1.0
    gap = (np.minimum(cu, 1 - cu) < 0.02) | (np.minimum(cv, 1 - cv) < 0.02)
    bus = (np.abs(cu - 0.33) < 0.006) | (np.abs(cu - 0.66) < 0.006)
    fingers = (cv * 40 % 1.0) < 0.06
    frame = (np.minimum(u, 1 - u) < 0.012) | (np.minimum(v, 1 - v) < 0.006)
    cell = np.array([0.010, 0.018, 0.060], dtype=np.float32) * (0.85 + 0.3 * noise(cells=64, seed=3, octaves=2))[..., None]
    diff = np.where(gap[..., None], np.float32(0.55), cell)
    diff = np.where((bus | fingers)[..., None] & ~gap[..., None], np.float32(0.35), diff)
    diff = np.where(frame[..., None], np.float32(0.6), diff)
    height = gap * -0.5 + frame * 1.0
    rough = np.where(gap | frame, 0.4, 0.12).astype(np.float32)
    return diff, normals_from_height(height.astype(np.float32), 3.0), rough


def stripes(a=(0.9, 0.55, 0.02), b=(0.02, 0.02, 0.02), count=4):
    """Diagonal hazard stripes, `count` pairs across a tile."""
    u, v = grid()
    s = ((u + v) * count) % 1.0 < 0.5
    diff = np.where(s[..., None], np.array(a, dtype=np.float32), np.array(b, dtype=np.float32))
    wear = noise(cells=16, seed=5)
    diff = diff * (0.85 + 0.2 * wear[..., None])
    return diff.astype(np.float32), None, (0.5 + 0.3 * wear).astype(np.float32)


def heat_tiles(seed=9):
    """Hexagonal heat-shield tiles, black with grey gaps."""
    u, v = grid()
    scale = 16.0
    x, y = u * scale, v * scale * 2 / np.sqrt(3)
    row = np.floor(y)
    x = x + (row % 2) * 0.5
    fx, fy = x % 1.0 - 0.5, y % 1.0 - 0.5
    edge = np.maximum(np.abs(fx) * 2, np.abs(fy) * 1.6 + np.abs(fx)) > 0.94
    tone = noise(cells=16, seed=seed)
    diff = np.where(edge[..., None], np.float32(0.12), (0.025 + 0.02 * tone)[..., None] * np.ones(3, dtype=np.float32))
    return diff.astype(np.float32), normals_from_height(-edge.astype(np.float32), 3.0), np.where(edge, 0.8, 0.55).astype(np.float32)
