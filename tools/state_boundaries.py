#!/usr/bin/env python3
"""Converts the Census Bureau's cartographic boundary file of the US states into the compact
file `crates/underbite-core/data/states/us-states.bin` that the core embeds.

Run once when the data is renewed; nothing fetches it at build or run time:

    curl -O https://www2.census.gov/geo/tiger/GENZ2025/shp/cb_2025_us_state_5m.zip
    mkdir states && cd states && unzip ../cb_2025_us_state_5m.zip && cd ..
    python3 -I tools/state_boundaries.py states/cb_2025_us_state_5m crates/underbite-core/data/states/us-states.bin

Only the Python standard library is used. The shapefile and dBase readers below handle just
what that file holds (polygons, character fields).

File format (all integers are unsigned LEB128 varints unless stated; "zigzag" is the signed
encoding n -> (n << 1) ^ (n >> 63)):

    magic            4 bytes "USB1"
    unit             varint: coordinates are whole multiples of 1/unit degree
    states           varint
    per state:
        code         2 bytes, the postal abbreviation
        rings        varint
        per ring (outer rings and holes alike; a point is inside when it is inside an odd
        number of rings):
            points   varint
            first    zigzag lon, zigzag lat, in units
            then     zigzag delta lon, zigzag delta lat from the previous point
"""
import struct
import sys

UNIT = 1000  # 1/1000 degree, about 110 m of latitude
TOLERANCE = 1.5  # Douglas-Peucker, in units (about 165 m)
MIN_EXTENT = 5  # rings smaller than this in both directions, in units, are dropped


def read_dbf(path):
    with open(path, "rb") as f:
        data = f.read()
    n, header_len, record_len = struct.unpack_from("<IHH", data, 4)
    fields = []
    pos = 32
    while data[pos] != 0x0D:
        name = data[pos:pos + 11].split(b"\0")[0].decode("ascii")
        length = data[pos + 16]
        fields.append((name, length))
        pos += 32
    rows = []
    for i in range(n):
        rec = data[header_len + i * record_len:header_len + (i + 1) * record_len]
        off = 1
        row = {}
        for name, length in fields:
            row[name] = rec[off:off + length].decode("latin-1").strip()
            off += length
        rows.append(row)
    return rows


def read_shp(path):
    with open(path, "rb") as f:
        data = f.read()
    pos = 100
    shapes = []
    while pos < len(data):
        (length,) = struct.unpack_from(">i", data, pos + 4)
        body = data[pos + 8:pos + 8 + length * 2]
        pos += 8 + length * 2
        (kind,) = struct.unpack_from("<i", body, 0)
        if kind == 0:
            shapes.append([])
            continue
        if kind != 5:
            raise SystemExit(f"unexpected shape type {kind}")
        nparts, npoints = struct.unpack_from("<ii", body, 36)
        parts = list(struct.unpack_from(f"<{nparts}i", body, 44)) + [npoints]
        pts = struct.unpack_from(f"<{2 * npoints}d", body, 44 + 4 * nparts)
        coords = [(pts[2 * i], pts[2 * i + 1]) for i in range(npoints)]
        shapes.append([coords[parts[i]:parts[i + 1]] for i in range(nparts)])
    return shapes


def dp(points, tol):
    """Douglas-Peucker on an open polyline of (x, y); iterative."""
    keep = [False] * len(points)
    keep[0] = keep[-1] = True
    stack = [(0, len(points) - 1)]
    while stack:
        a, b = stack.pop()
        (ax, ay), (bx, by) = points[a], points[b]
        dx, dy = bx - ax, by - ay
        norm = dx * dx + dy * dy
        best, at = -1.0, -1
        for i in range(a + 1, b):
            px, py = points[i]
            if norm == 0:
                d = (px - ax) ** 2 + (py - ay) ** 2
            else:
                t = max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / norm))
                d = (px - ax - t * dx) ** 2 + (py - ay - t * dy) ** 2
            if d > best:
                best, at = d, i
        if best > tol * tol:
            keep[at] = True
            stack.append((a, at))
            stack.append((at, b))
    return [p for p, k in zip(points, keep) if k]


def ring_units(ring):
    pts = [(round(x * UNIT), round(y * UNIT)) for x, y in ring]
    out = [pts[0]]
    for p in pts[1:]:
        if p != out[-1]:
            out.append(p)
    return out


def simplify(ring):
    pts = ring_units(ring)
    if pts[0] == pts[-1]:
        pts = pts[:-1]
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    if max(xs) - min(xs) < MIN_EXTENT and max(ys) - min(ys) < MIN_EXTENT:
        return None
    if len(pts) < 4:
        return pts if len(pts) >= 3 else None
    # Split the closed ring at its two farthest-apart extreme points so both halves simplify.
    i = xs.index(min(xs))
    j = xs.index(max(xs))
    if i == j:
        return None
    if i > j:
        i, j = j, i
    a = dp(pts[i:j + 1], TOLERANCE)
    b = dp(pts[j:] + pts[:i + 1], TOLERANCE)
    out = a[:-1] + b[:-1]
    return out if len(out) >= 3 else None


def varint(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def zigzag(n):
    return varint((n << 1) ^ (n >> 63))


def main(stem, out_path):
    rows = read_dbf(stem + ".dbf")
    shapes = read_shp(stem + ".shp")
    assert len(rows) == len(shapes)
    states = []
    for row, rings in zip(rows, shapes):
        code = row["STUSPS"]
        kept = [r for r in (simplify(r) for r in rings) if r]
        states.append((code, kept))
    states.sort()
    out = bytearray(b"USB1")
    out += varint(UNIT)
    out += varint(len(states))
    for code, rings in states:
        out += code.encode("ascii")
        out += varint(len(rings))
        for ring in rings:
            out += varint(len(ring))
            px = py = 0
            for k, (x, y) in enumerate(ring):
                out += zigzag(x - px) + zigzag(y - py)
                px, py = x, y
    with open(out_path, "wb") as f:
        f.write(out)
    print(f"{len(states)} states, {sum(len(r) for _, rs in states for r in rs)} points, "
          f"{len(out)} bytes")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
