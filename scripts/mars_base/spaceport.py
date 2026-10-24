"""The spaceport: sintered landing pads ringed by blast walls and lights, a
Starship on one and a crew lander on the other, and their propellant."""
import math

import bmesh
from mathutils import Vector

from buildings import light_mast, mats
from kit import (TAU, Building, box, disc, lathe, merge, moved, rod, sphere, sweep)


def landing_pad(looks, name, radius=32.0, at=(0, 0, 0), parent=None, walls=10):
    """A sintered-regolith pad: paved disc, painted target, edge lights, a
    blast wall broken by gaps for the crawlers."""
    m = mats(looks)
    b = Building(name, parent, at, 0.0)
    b.put(lathe([(radius + 1.0, -0.1), (radius + 1.0, 0.04), (radius, 0.07), (0.0, 0.07)], 128, 3.0), m["pad"], sharp=40)
    for r0, r1 in ((radius * 0.62, radius * 0.62 - 0.8), (radius * 0.98, radius * 0.98 - 0.5), (4.0, 3.2)):
        b.put(disc(r0, 0.075, 128, 2.0, r1), m["paint"])
    for a in (0.0, math.pi / 2):
        for s in (-1, 1):
            b.put(moved(box((radius * 0.25, 0.8, 0.02), (s * radius * 0.4, 0, 0.075)), (0, 0, 0), a), m["paint"], smooth=False)
    for i in range(32):
        a = TAU * i / 32
        c = Vector((math.cos(a), math.sin(a), 0)) * (radius + 0.5)
        b.put(moved(lathe([(0.0, 0.0), (0.18, 0.0), (0.18, 0.35), (0.0, 0.4)], 10, 1.0), tuple(c)), m["green"] if i % 2 else m["red"])
    gap = 0.18
    for k in range(walls):
        a0 = TAU * k / walls + gap
        a1 = TAU * (k + 1) / walls - gap
        arc = [(radius + 9.0) * Vector((math.cos(a), math.sin(a), 0)) for a in [a0 + (a1 - a0) * i / 12 for i in range(13)]]
        b.put(sweep([(0.0, -1.8), (0.0, 1.8), (3.2, 0.5), (3.2, -0.5)], arc, 1.2), m["concrete"], sharp=30)
    for i in range(4):
        a = TAU * i / 4 + TAU / 8
        c = Vector((math.cos(a), math.sin(a), 0)) * (radius + 4.0)
        light_mast(b, m, tuple(c), 12.0, 3, a + math.pi)
    for k in range(walls):
        a0, a1 = TAU * k / walls + gap, TAU * (k + 1) / walls - gap
        arc = [(radius + 9.0) * Vector((math.cos(a), math.sin(a), 0)) for a in (a0, (a0 + a1) / 2, a1)]
        b.collider(sweep([(0.0, -1.8), (0.0, 1.8), (3.2, 0.5), (3.2, -0.5)], arc, 1.2))
    return b.finish()


def starship(looks, name, at=(0, 0, 0), yaw=0.0, parent=None):
    """A stainless Starship standing on its legs: ring-welded barrel, ogive
    nose, heat tiles on its windward side, flaps fore and aft, Raptors below."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    r, z0, body = 4.5, 3.2, 36.0
    profile = [(0.0, z0), (r * 0.95, z0), (r, z0 + 0.4), (r, z0 + body)]
    nose = [(r * math.cos(t * math.pi / 2) ** 0.75, z0 + body + 12.0 * math.sin(t * math.pi / 2)) for t in [i / 16 for i in range(1, 17)]]
    nose[-1] = (0.0, z0 + body + 12.0)
    b.put(lathe(profile + nose, 72, 3.0), m["stainless"], sharp=30)
    tiles = [(r + 0.03, z0 + 0.6), (r + 0.03, z0 + body)] + [(rr + 0.03, zz) for rr, zz in nose[:-2]]
    b.put(lathe(tiles, 36, 1.6, angle=math.pi, start=math.pi / 2), m["tiles"], sharp=30)
    for s in (-1, 1):
        fwd = box((0.3, 2.6, 5.0), (0, s * (r + 1.2), z0 + body + 3.0), bevel=0.12)
        b.put(fwd, m["tiles"], smooth=False)
        aft = box((0.4, 3.6, 9.0), (0, s * (r + 1.7), z0 + 5.5), bevel=0.15)
        b.put(aft, m["tiles"], smooth=False)
    b.put(lathe([(r, z0 + 0.4), (r * 1.02, z0 - 0.6), (r * 0.9, z0 - 1.0)], 72, 2.0), m["stainless"], sharp=30)
    for i in range(6):
        a = TAU * i / 6 + TAU / 12
        c = Vector((math.cos(a), math.sin(a), 0))
        b.put(rod(c * (r - 0.3) + Vector((0, 0, z0 + 2.0)), c * (r + 1.6) + Vector((0, 0, 0.3)), 0.25, 10), m["stainless"])
        b.put(moved(lathe([(0.0, 0.0), (0.9, 0.0), (0.9, 0.18), (0.3, 0.4), (0.0, 0.4)], 16, 1.0), tuple(c * (r + 1.6))), m["dark"])
    for i in range(6):
        a = TAU * i / 6
        c = Vector((math.cos(a), math.sin(a), 0)) * (1.2 if i % 2 else 2.8)
        bell = lathe([(0.0, 0.0), (0.65 if i % 2 else 1.1, 0.0), (0.35, 1.6), (0.3, 2.2), (0.0, 2.3)], 24, 1.0)
        b.put(moved(bell, tuple(c + Vector((0, 0, z0 - 1.9)))), m["dark"])
    b.put(moved(box((0.08, 1.2, 1.9), (0, 0, 0), bevel=0.2), (r + 0.01, 0, z0 + body - 6.0)), m["hull_grey"], smooth=False)
    for k in range(3):
        b.put(sphere(0.25, (r * math.cos(0.4 * k - 0.4), r * math.sin(0.4 * k - 0.4), z0 + body - 2.0), 8, 4), m["window"])
    b.collider(lathe([(0.0, z0 - 1.0), (r, z0), (r, z0 + body), (0.0, z0 + body + 12.0)], 16, 4.0))
    for i in range(6):
        a = TAU * i / 6 + TAU / 12
        foot = Vector((math.cos(a), math.sin(a), 0)) * (r + 1.6)
        b.collider(box((0.9, 0.9, z0), (foot.x, foot.y, z0 / 2)))
    return b.finish()


def crew_lander(looks, name, at=(0, 0, 0), yaw=0.0, parent=None):
    """A crew lander in the manner of NASA's reference missions: a white
    conical cabin over a descent stage on four legs, a ladder down."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    b.put(lathe([(0.0, 2.2), (3.4, 2.2), (3.6, 3.6), (3.4, 4.0), (0.0, 4.0)], 8, 2.0, start=math.pi / 8), m["hull_grey"], smooth=False)
    b.put(lathe([(3.0, 4.0), (2.2, 9.5), (1.2, 10.6), (0.0, 10.8)], 48, 2.0), m["hull"], sharp=40)
    for i in range(4):
        a = TAU * i / 4 + TAU / 8
        c = Vector((math.cos(a), math.sin(a), 0))
        b.put(moved(sphere(1.0, (0, 0, 0), 16, 8), tuple(c * 2.2 + Vector((0, 0, 3.0)))), m["gold"])
        b.put(rod(c * 3.2 + Vector((0, 0, 2.6)), c * 5.4 + Vector((0, 0, 0.3)), 0.16, 10), m["steel"])
        b.put(rod(c * 2.6 + Vector((0, 0, 2.2)), c * 5.4 + Vector((0, 0, 0.3)), 0.08, 8), m["steel"])
        b.put(moved(lathe([(0.0, 0.0), (0.7, 0.0), (0.7, 0.12), (0.2, 0.3), (0.0, 0.3)], 16, 1.0), tuple(c * 5.4)), m["dark"])
    for k in range(4):
        b.put(moved(lathe([(0.0, 0.0), (0.5, 0.0), (0.25, 1.3), (0.0, 1.4)], 16, 1.0), (math.cos(TAU * k / 4) * 1.2, math.sin(TAU * k / 4) * 1.2, 0.9)), m["dark"])
    b.put(moved(box((0.15, 1.0, 1.6), (0, 0, 0), bevel=0.15), (2.75, 0, 5.2)), m["hull_grey"], smooth=False)
    for s in (-0.3, 0.3):
        b.put(rod((3.4, s, 4.4), (5.0, s, 0.1), 0.03, 6), m["yellow"])
    rungs = bmesh.new()
    for k in range(12):
        t = k / 12
        p = Vector((3.4, 0, 4.4)).lerp(Vector((5.0, 0, 0.1)), t)
        merge(rungs, rod(p + Vector((0, -0.3, 0)), p + Vector((0, 0.3, 0)), 0.02, 6))
    b.put(rungs, m["frame"])
    for k in range(3):
        b.put(sphere(0.22, (2.45 * math.cos(0.5 * k - 0.5), 2.45 * math.sin(0.5 * k - 0.5), 8.0), 8, 4), m["window"])
    b.collider(lathe([(0.0, 2.2), (3.6, 2.2), (3.6, 4.0), (2.2, 9.5), (0.0, 10.8)], 12, 4.0))
    for i in range(4):
        a = TAU * i / 4 + TAU / 8
        foot = Vector((math.cos(a), math.sin(a), 0)) * 5.4
        b.collider(box((0.8, 0.8, 2.4), (foot.x, foot.y, 1.2)))
    return b.finish()
