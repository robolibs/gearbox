"""In-situ resource use at the scale one cargo lander can bring: container-
sized ISRU units on their landing frames, a small set of propellant and
water tanks, an ice well, regolith dug by excavators, pipes on sleepers."""
import math
import random

import bmesh
from mathutils import Vector

from buildings import light_mast, mats, railing, unit
from kit import (TAU, Building, box, lathe, merge, moved, rect, rod, sphere, sweep, truss, tube)


def _sphere_tank(b, m, at, r, skin):
    at = Vector(at)
    b.put(sphere(r, tuple(at + Vector((0, 0, r + 0.8))), 32, 16), skin)
    b.put(moved(lathe([(r * 1.01, -0.1), (r * 1.01, 0.1)], 32, 1.0), tuple(at + Vector((0, 0, r + 0.8)))), m["hull_grey"])
    for i in range(4):
        a = TAU * i / 4 + TAU / 8
        c = Vector((math.cos(a), math.sin(a), 0)) * r * 0.9
        b.put(rod(at + c, at + c + Vector((0, 0, r + 0.8)), 0.08, 8), m["steel"])
        b.put(moved(box((0.4, 0.4, 0.12), (0, 0, 0.06)), tuple(at + c)), m["dark"], smooth=False)


def _bullet_tank(b, m, at, yaw, length, r, skin):
    """A horizontal pressure vessel with domed heads on two saddles."""
    shell = lathe([(0.0, 0.0), (r * 0.7, 0.12 * r), (r, 0.45 * r)] + [(r, length - 0.45 * r), (r * 0.7, length - 0.12 * r), (0.0, length)], 32, 2.0)
    b.put(moved(shell, tuple(Vector(at) + Vector((0, 0, r + 0.4))), yaw, math.pi / 2), skin, sharp=40)
    for t in (0.22, 0.78):
        d = Vector((math.cos(yaw), math.sin(yaw), 0)) * (length * t)
        b.put(moved(box((0.4, r * 1.6, r * 0.7), (0, 0, r * 0.35), tile=1.0), tuple(Vector(at) + d), yaw), m["steel"], smooth=False)


def isru_unit(looks, name, at=(0, 0, 0), yaw=0.0, parent=None):
    """One landed ISRU unit, a Sabatier and electrolysis plant in a 6 m
    shell on the frame it landed on: CO2 intakes, a cryocooler, radiator
    wings either side, valves and a hose bay."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    L, W, H = 6.0, 2.4, 2.5
    z0 = 0.7
    b.put(box((L, W, H), (0, 0, z0 + H / 2), tile=2.0, bevel=0.06), m["hull"], smooth=False)
    b.put(box((L + 0.1, W + 0.1, 0.12), (0, 0, z0 + H - 0.5), tile=1.0), m["hull_orange"], smooth=False)
    frame = bmesh.new()
    merge(frame, box((L + 0.4, W + 0.4, 0.2), (0, 0, z0 - 0.1)))
    for sx in (-1, 1):
        for sy in (-1, 1):
            foot = Vector((sx * (L / 2 + 0.6), sy * (W / 2 + 0.6), 0.08))
            merge(frame, rod(Vector((sx * L / 2, sy * W / 2, z0 - 0.1)), foot, 0.07, 8))
            merge(frame, moved(lathe([(0.0, 0.0), (0.35, 0.0), (0.35, 0.08), (0.0, 0.12)], 12, 1.0), tuple(foot - Vector((0, 0, 0.08)))))
    b.put(frame, m["steel"])
    for k in range(3):
        x = -L / 2 + 1.0 + k * 1.2
        b.put(moved(lathe([(0.0, 0.0), (0.3, 0.0), (0.3, 0.5), (0.36, 0.55), (0.0, 0.6)], 16, 1.0), (x, -0.4, z0 + H)), m["grate"])
    unit(b, m, (L / 2 - 1.3, 0.3, z0 + H), 0.0, (1.6, 1.2, 0.6), 1)
    for s in (-1, 1):
        wing = Vector((0, s * (W / 2 + 0.15), z0 + H - 0.3))
        b.put(rod(wing, wing + Vector((0, s * 0.6, 0.6)), 0.05, 6), m["steel"])
        b.put(moved(box((L - 0.6, 0.05, 2.2), (0, 0, 0)), tuple(wing + Vector((0, s * 1.4, 1.0))), 0.0, 0.0, s * 0.5), m["radiator"], smooth=False)
    for k in range(3):
        b.put(moved(box((0.06, 0.5, 0.5), (0, 0, 0), bevel=0.02), (L / 2 + 0.03, -0.7 + k * 0.7, z0 + 1.0)), m["hull_grey"], smooth=False)
        b.put(tube([(L / 2 + 0.05, -0.7 + k * 0.7, z0 + 1.0), (L / 2 + 0.6, -0.7 + k * 0.7, z0 + 1.0), (L / 2 + 0.9, -0.7 + k * 0.7, 0.08)], 0.06, 8),
              [m["pipe_yellow"], m["pipe_white"], m["pipe_blue"]][k])
    b.places.append(("light", {"at": (L / 2 + 0.5, 0, z0 + H + 0.2), "kind": "door"}))
    b.collider(box((L + 0.4, W + 0.4, z0 + H), (0, 0, (z0 + H) / 2)))
    return b.finish()


def propellant_tanks(looks, name, at=(0, 0, 0), yaw=0.0, parent=None):
    """The ISRU products, stored: liquid oxygen and methane in two spheres,
    water in two bullets, on a gravel pad inside a low bund."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    b.put(box((18.0, 12.0, 0.08), (0, 0, 0.0), tile=2.0), m["pad"], smooth=False)
    for s in (-1, 1):
        b.put(box((18.0, 0.3, 0.45), (0, s * 6.0, 0.22), tile=1.0), m["bags"], smooth=False)
        b.put(box((0.3, 12.0, 0.45), (s * 9.0, 0, 0.22), tile=1.0), m["bags"], smooth=False)
    _sphere_tank(b, m, (-4.5, -2.6, 0), 2.0, m["gold"])
    _sphere_tank(b, m, (-4.5, 2.6, 0), 2.0, m["hull"])
    for k in range(2):
        _bullet_tank(b, m, (0.5, -1.6 + k * 3.2, 0), 0.0, 6.5, 1.1, m["hull"])
    for k, colour in enumerate(("pipe_yellow", "pipe_white", "pipe_blue")):
        y = -1.0 + k * 1.0
        b.put(tube([(-7.0, y, 0.35), (8.6, y, 0.35)], 0.07, 8), m[colour])
    for x in range(-6, 9, 3):
        b.put(box((0.25, 2.6, 0.25), (x, 0, 0.14)), m["dark"], smooth=False)
    light_mast(b, m, (8.0, 5.0, 0), 7.0, 2, math.pi * 1.25)
    for y in (-2.6, 2.6):
        b.collider(moved(lathe([(0.0, 0.0), (2.0, 0.0), (2.0, 4.8), (0.0, 4.8)], 10, 2.0), (-4.5, y, 0)))
    b.collider(box((6.5, 5.4, 2.6), (3.75, 0, 1.3)))
    for s in (-1, 1):
        b.collider(box((18.0, 0.3, 0.45), (0, s * 6.0, 0.22)))
        b.collider(box((0.3, 12.0, 0.45), (s * 9.0, 0, 0.22)))
    return b.finish()


def pipe_run(looks, name, path, parent=None, colours=("pipe_white", "pipe_yellow", "pipe_blue")):
    """Insulated pipes on low sleepers along a path, as laid on the ground."""
    m = mats(looks)
    path = [Vector(p) for p in path]
    b = Building(name, parent, (0, 0, 0), 0.0)
    for a, c in zip(path, path[1:]):
        run = c - a
        side = Vector((-run.y, run.x, 0)).normalized()
        n = max(1, int(run.length / 4.0))
        sleepers = bmesh.new()
        for k in range(n + 1):
            p = a + run * (k / n)
            merge(sleepers, moved(box((0.25, 1.4, 0.22), (0, 0, 0.11)), tuple(p), math.atan2(run.y, run.x)))
        b.put(sleepers, m["dark"], smooth=False)
    for i, colour in enumerate(colours):
        off = -0.4 + 0.4 * i
        pts = []
        for k, p in enumerate(path):
            d = (path[min(k + 1, len(path) - 1)] - path[max(k - 1, 0)])
            side = Vector((-d.y, d.x, 0)).normalized()
            pts.append(p + side * off + Vector((0, 0, 0.32)))
        b.put(tube(pts, 0.08, 10), m[colour])
    return b.finish()


def drill_rig(looks, name, at=(0, 0, 0), yaw=0.0, parent=None, height=9.0):
    """An ice well, after the Rodwell: a compact drill mast on a skid, its
    melt and pump house beside it, and the heated line back to the plant."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    b.put(box((6.0, 4.0, 0.6), (0, 0, 0.3), tile=1.0), m["hull_grey"], smooth=False)
    b.put(box((6.0, 4.0, 0.05), (0, 0, 0.62), tile=1.0), m["grate"], smooth=False)
    legs = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
    top = [(x * 0.35, y * 0.35) for x, y in legs]
    der = bmesh.new()
    for (x0, y0), (x1, y1) in zip(legs, top):
        merge(der, rod((x0, y0, 0.6), (x1, y1, height), 0.08, 8))
    for k in range(6):
        t0, t1 = k / 6, (k + 1) / 6
        def p(q, t):
            return Vector((legs[q][0] + (top[q][0] - legs[q][0]) * t, legs[q][1] + (top[q][1] - legs[q][1]) * t, 0.6 + (height - 0.6) * t))
        for i in range(4):
            j = (i + 1) % 4
            merge(der, rod(p(i, t0), p(j, t0), 0.04, 6))
            merge(der, rod(p(i, t0), p(j, t1), 0.035, 6))
    b.put(der, m["hull_orange"])
    b.put(box((0.9, 0.9, 0.6), (0, 0, height + 0.3), bevel=0.04), m["yellow"], smooth=False)
    b.put(rod((0, 0, 0.6), (0, 0, height - 1.0), 0.09, 10), m["steel"])
    b.put(box((2.4, 2.0, 2.0), (-1.6, 0.0, 1.6), tile=1.0, bevel=0.05), m["hull"], smooth=False)
    b.put(tube([(-1.6, 1.0, 1.0), (-1.6, 2.6, 0.2), (8.0, 4.0, 0.15)], 0.08, 8), m["pipe_blue"])
    railing(b, m, [(1.0, -2.0, 0.62), (3.0, -2.0, 0.62), (3.0, 2.0, 0.62), (1.0, 2.0, 0.62)], 1.0, 1.5)
    b.put(sphere(0.15, (0, 0, height + 0.8), 10, 5), m["red"])
    b.collider(box((6.0, 4.0, 0.62), (0, 0, 0.31)))
    b.collider(box((2.0, 2.0, height), (0, 0, height / 2)))
    b.collider(box((2.4, 2.0, 2.6), (-1.6, 0.0, 1.3)))
    return b.finish()


def regolith_works(looks, name, at=(0, 0, 0), yaw=0.0, parent=None, seed=3):
    """Regolith the excavators dig: spoil heaps, and a feed hopper with a
    short conveyor up to the plant's oven."""
    m = mats(looks)
    b = Building(name, parent, at, yaw)
    rng = random.Random(seed)
    for k in range(3):
        x, y = rng.uniform(-10, 0), rng.uniform(-6, 6)
        r, h = rng.uniform(2.5, 4.0), rng.uniform(0.9, 1.6)
        profile = [(r * (1.0 - t), h * (1.0 - (1.0 - t) ** 1.8)) for t in [i / 10 for i in range(11)]]
        profile[-1] = (0.0, h)
        heap = lathe(profile, 40, 2.0)
        for v in heap.verts:
            n = 0.18 * math.sin(v.co.x * 1.3 + k) * math.cos(v.co.y * 1.1 - k) + 0.1 * math.sin(v.co.x * 3.1 + v.co.y * 2.3)
            v.co.z = max(0.0, v.co.z + n * min(1.0, v.co.z))
        b.put(moved(heap, (x, y, -0.05)), m["regolith"])
        b.collider(moved(lathe([(r, 0.0), (r * 0.5, h * 0.8), (0.0, h)], 10, 2.0), (x, y, -0.05)))
    hop = lathe([(0.3, 1.4), (1.1, 2.8), (1.1, 3.1), (0.0, 3.1)], 4, 1.0, start=math.pi / 4)
    b.put(moved(hop, (4.0, 0, 0)), m["hull_orange"], smooth=False)
    for i in range(4):
        a = math.pi / 4 + TAU * i / 4
        c = Vector((math.cos(a), math.sin(a), 0)) * 0.8
        b.put(rod(c + Vector((4.0, 0, 0)), c + Vector((4.0, 0, 2.8)), 0.06, 8), m["steel"])
    b.put(truss((4.6, 0, 1.4), (12.0, 0, 3.6), 0.7, 0.6, 5, 0.04, 0.025), m["steel"])
    b.put(sweep(rect(0.03, 0.5), [(4.6, 0, 1.75), (12.0, 0, 3.95)], 1.0), m["rubber"])
    b.put(rod(Vector((8.3, 0, 0)), Vector((8.3, 0, 2.2)), 0.06, 8), m["steel"])
    return b.finish()
