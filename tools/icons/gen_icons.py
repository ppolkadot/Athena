#!/usr/bin/env python3
"""Regenerate src/ui/icon_paths.rs from the vendored Lucide 0.468.0 SVGs.

Usage: python tools/icons/gen_icons.py

Reads assets/icons/lucide/<name>.svg (24-unit viewBox, 2-unit stroke) and flattens every
path, circle, rect and line into polylines in 24-unit coordinates. Curves are flattened with
fixed angular/parametric steps, so the output is deterministic. A polyline whose first and last
point coincide is a closed shape.
"""
import math
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "assets" / "icons" / "lucide"
OUT = ROOT / "src" / "ui" / "icon_paths.rs"

# (Rust constant, lucide file stem)
ICONS = [
    ("LIST", "list"),
    ("CALENDAR_DAYS", "calendar-days"),
    ("TRASH_2", "trash-2"),
    ("SETTINGS", "settings"),
    ("CHEVRON_LEFT", "chevron-left"),
    ("CHEVRON_RIGHT", "chevron-right"),
    ("PLUS", "plus"),
    ("X", "x"),
    ("PENCIL", "pencil"),
    ("ARROW_DOWN_LEFT", "arrow-down-left"),
    ("ARROW_UP_RIGHT", "arrow-up-right"),
    ("UNDO_2", "undo-2"),
    ("ARROW_LEFT_RIGHT", "arrow-left-right"),
    ("TARGET", "target"),
    ("SEARCH", "search"),
    ("BOOKMARK", "bookmark"),
    ("TRIANGLE_ALERT", "triangle-alert"),
]

NUM = re.compile(r"[-+]?(?:\d*\.\d+|\d+\.?)(?:[eE][-+]?\d+)?")


def tokens(d):
    for m in re.finditer(r"([MmLlHhVvCcSsQqTtAaZz])|([-+]?(?:\d*\.\d+|\d+\.?)(?:[eE][-+]?\d+)?)", d):
        yield m.group(1) or float(m.group(2))


def arc_points(p0, rx, ry, phi_deg, large, sweep, p1):
    x1, y1 = p0
    x2, y2 = p1
    if (x1, y1) == (x2, y2):
        return []
    rx, ry = abs(rx), abs(ry)
    if rx == 0 or ry == 0:
        return [p1]
    phi = math.radians(phi_deg)
    cp, sp = math.cos(phi), math.sin(phi)
    dx, dy = (x1 - x2) / 2, (y1 - y2) / 2
    x1p = cp * dx + sp * dy
    y1p = -sp * dx + cp * dy
    lam = (x1p ** 2) / (rx ** 2) + (y1p ** 2) / (ry ** 2)
    if lam > 1:
        s = math.sqrt(lam)
        rx, ry = rx * s, ry * s
    num = rx ** 2 * ry ** 2 - rx ** 2 * y1p ** 2 - ry ** 2 * x1p ** 2
    den = rx ** 2 * y1p ** 2 + ry ** 2 * x1p ** 2
    coef = math.sqrt(max(0.0, num / den)) if den else 0.0
    if large == sweep:
        coef = -coef
    cxp = coef * rx * y1p / ry
    cyp = -coef * ry * x1p / rx
    cx = cp * cxp - sp * cyp + (x1 + x2) / 2
    cy = sp * cxp + cp * cyp + (y1 + y2) / 2

    def ang(ux, uy, vx, vy):
        a = math.atan2(ux * vy - uy * vx, ux * vx + uy * vy)
        return a

    th1 = ang(1, 0, (x1p - cxp) / rx, (y1p - cyp) / ry)
    dth = ang((x1p - cxp) / rx, (y1p - cyp) / ry, (-x1p - cxp) / rx, (-y1p - cyp) / ry)
    if not sweep and dth > 0:
        dth -= 2 * math.pi
    elif sweep and dth < 0:
        dth += 2 * math.pi
    steps = max(2, math.ceil(abs(dth) / math.radians(15)))
    pts = []
    for i in range(1, steps + 1):
        t = th1 + dth * i / steps
        x = cp * rx * math.cos(t) - sp * ry * math.sin(t) + cx
        y = sp * rx * math.cos(t) + cp * ry * math.sin(t) + cy
        pts.append((x, y))
    pts[-1] = p1
    return pts


def bezier(p0, c1, c2, p1, n=10):
    pts = []
    for i in range(1, n + 1):
        t = i / n
        u = 1 - t
        x = u ** 3 * p0[0] + 3 * u * u * t * c1[0] + 3 * u * t * t * c2[0] + t ** 3 * p1[0]
        y = u ** 3 * p0[1] + 3 * u * u * t * c1[1] + 3 * u * t * t * c2[1] + t ** 3 * p1[1]
        pts.append((x, y))
    return pts


def quad(p0, c, p1, n=8):
    pts = []
    for i in range(1, n + 1):
        t = i / n
        u = 1 - t
        pts.append((u * u * p0[0] + 2 * u * t * c[0] + t * t * p1[0],
                    u * u * p0[1] + 2 * u * t * c[1] + t * t * p1[1]))
    return pts


def parse_path(d):
    toks = list(tokens(d))
    i = 0
    subs = []
    cur = None
    pos = (0.0, 0.0)
    start = (0.0, 0.0)
    cmd = None
    last_c = None  # last cubic control for S
    last_q = None  # last quad control for T

    def num():
        nonlocal i
        v = toks[i]
        i += 1
        return v

    def flag():
        # arc flags may be packed ("a2 2 0 011 1"); the vendored set never packs them,
        # but accept single-digit tokens
        return int(num())

    while i < len(toks):
        if isinstance(toks[i], str):
            cmd = toks[i]
            i += 1
            if cmd in "Zz":
                if cur:
                    cur.append(start)
                    subs.append(cur)
                    cur = None
                pos = start
                last_c = last_q = None
                continue
        rel = cmd.islower()
        c = cmd.upper()
        ox, oy = pos if rel else (0.0, 0.0)
        if c == "M":
            x, y = num() + ox, num() + oy
            if cur:
                subs.append(cur)
            pos = start = (x, y)
            cur = [pos]
            cmd = "l" if rel else "L"
            last_c = last_q = None
        elif c == "L":
            pos = (num() + ox, num() + oy)
            cur.append(pos)
            last_c = last_q = None
        elif c == "H":
            pos = (num() + ox, pos[1])
            cur.append(pos)
            last_c = last_q = None
        elif c == "V":
            pos = (pos[0], num() + oy)
            cur.append(pos)
            last_c = last_q = None
        elif c == "C":
            c1 = (num() + ox, num() + oy)
            c2 = (num() + ox, num() + oy)
            p1 = (num() + ox, num() + oy)
            cur.extend(bezier(pos, c1, c2, p1))
            pos, last_c, last_q = p1, c2, None
        elif c == "S":
            c1 = (2 * pos[0] - last_c[0], 2 * pos[1] - last_c[1]) if last_c else pos
            c2 = (num() + ox, num() + oy)
            p1 = (num() + ox, num() + oy)
            cur.extend(bezier(pos, c1, c2, p1))
            pos, last_c, last_q = p1, c2, None
        elif c == "Q":
            c1 = (num() + ox, num() + oy)
            p1 = (num() + ox, num() + oy)
            cur.extend(quad(pos, c1, p1))
            pos, last_q, last_c = p1, c1, None
        elif c == "T":
            c1 = (2 * pos[0] - last_q[0], 2 * pos[1] - last_q[1]) if last_q else pos
            p1 = (num() + ox, num() + oy)
            cur.extend(quad(pos, c1, p1))
            pos, last_q, last_c = p1, c1, None
        elif c == "A":
            rx, ry, rot = num(), num(), num()
            large, sweep = flag(), flag()
            p1 = (num() + ox, num() + oy)
            cur.extend(arc_points(pos, rx, ry, rot, large, sweep, p1))
            pos = p1
            last_c = last_q = None
        else:
            raise ValueError(f"unsupported command {cmd}")
    if cur:
        subs.append(cur)
    return subs


def circle(cx, cy, r, n=32):
    pts = [(cx + r * math.cos(2 * math.pi * k / n), cy + r * math.sin(2 * math.pi * k / n)) for k in range(n)]
    pts.append(pts[0])
    return pts


def rect(x, y, w, h, rx):
    if rx <= 0:
        return [(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y)]
    d = (f"M{x + rx} {y}H{x + w - rx}A{rx} {rx} 0 0 1 {x + w} {y + rx}V{y + h - rx}"
         f"A{rx} {rx} 0 0 1 {x + w - rx} {y + h}H{x + rx}A{rx} {rx} 0 0 1 {x} {y + h - rx}"
         f"V{y + rx}A{rx} {rx} 0 0 1 {x + rx} {y}Z")
    return parse_path(d)[0]


def shapes(svg: Path):
    root = ET.parse(svg).getroot()
    out = []
    for el in root:
        tag = el.tag.split("}")[-1]
        a = el.attrib
        f = lambda k, default=0.0: float(a.get(k, default))
        if tag == "path":
            out.extend(parse_path(a["d"]))
        elif tag == "circle":
            out.append(circle(f("cx"), f("cy"), f("r")))
        elif tag == "rect":
            out.append(rect(f("x"), f("y"), f("width"), f("height"), f("rx")))
        elif tag == "line":
            out.append([(f("x1"), f("y1")), (f("x2"), f("y2"))])
        elif tag == "polyline":
            vals = [float(v) for v in NUM.findall(a["points"])]
            out.append(list(zip(vals[0::2], vals[1::2])))
        else:
            raise ValueError(f"{svg.name}: unsupported element {tag}")
    return out


def fmt(v):
    s = f"{v:.3f}".rstrip("0").rstrip(".")
    if s in ("", "-0"):
        s = "0"
    return s if "." in s else s + ".0"


def main():
    lines = [
        "//! Generated by `tools/icons/gen_icons.py` from Lucide 0.468.0 (ISC, see assets/icons/LICENSE.txt).",
        "//! Do not edit by hand. Polylines in 24-unit coordinates; a polyline whose last point equals its",
        "//! first is closed. Painted with a 2-unit round stroke by `ui::icons`.",
        "",
        "pub type Poly = &'static [[f32; 2]];",
        "",
    ]
    for const, stem in ICONS:
        polys = shapes(SRC / f"{stem}.svg")
        lines.append(f"/// lucide `{stem}`")
        lines.append(f"pub const {const}: &[Poly] = &[")
        for poly in polys:
            pts = ", ".join(f"[{fmt(x)}, {fmt(y)}]" for x, y in poly)
            lines.append(f"    &[{pts}],")
        lines.append("];")
        lines.append("")
    OUT.write_text("\n".join(lines), encoding="utf-8", newline="\n")
    print(f"wrote {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    sys.exit(main())
