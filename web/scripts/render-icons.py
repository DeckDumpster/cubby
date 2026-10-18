"""Renders the Cubby mark (icons-src/mark.svg geometry) to static/assets/icons/. Stdlib only.

Rounded mark: favicon and PWA icons. Full-bleed mark: apple-touch (iOS rounds it, and it must be
opaque) and the maskable icon. Keep the constants in step with icons-src/*.svg.
"""
import math, os, struct, sys, zlib

os.chdir(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
OUT = "static/assets/icons"
ACCENT = (0x4F, 0x46, 0xE5)
R, W, HALF_GAP, RX = 120.0, 64.0, math.radians(40), 112.0
END = [(256 + R * math.cos(HALF_GAP), 256 - R * math.sin(HALF_GAP)), (256 + R * math.cos(HALF_GAP), 256 + R * math.sin(HALF_GAP))]


def in_rect(x, y):
    dx, dy = abs(x - 256) - (256 - RX), abs(y - 256) - (256 - RX)
    return max(dx, 0) ** 2 + max(dy, 0) ** 2 <= RX * RX


def in_c(x, y):
    dx, dy = x - 256, y - 256
    if abs(math.atan2(dy, dx)) >= HALF_GAP:
        return abs(math.hypot(dx, dy) - R) <= W / 2
    return any(math.hypot(x - ex, y - ey) <= W / 2 for ex, ey in END)


def render(size, rounded, ss=4):
    rows = []
    for py in range(size):
        row = bytearray()
        for px in range(size):
            cov = bg = 0
            for sy in range(ss):
                for sx in range(ss):
                    x, y = (px + (sx + .5) / ss) * 512 / size, (py + (sy + .5) / ss) * 512 / size
                    if in_rect(x, y) or not rounded:
                        bg += 1
                        cov += in_c(x, y)
            a = bg / ss / ss
            if a == 0:
                row += bytes((0, 0, 0, 0))
                continue
            t = cov / bg
            row += bytes([round(c * (1 - t) + 255 * t) for c in ACCENT] + [round(255 * a)])
        rows.append(bytes(row))
    return rows


def png(size, rounded):
    rows = render(size, rounded)
    if not rounded:  # opaque RGB
        rows = [bytes(v for i, v in enumerate(r) if i % 4 != 3) for r in rows]
    ct, bpp = (6, 4) if rounded else (2, 3)
    raw = b"".join(b"\0" + r for r in rows)
    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, ct, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def write(name, data):
    open(f"{OUT}/{name}", "wb").write(data)


os.makedirs(OUT, exist_ok=True)
write("favicon-v1.svg", open("icons-src/mark.svg", "rb").read())
write("apple-touch-icon-v1.png", png(180, False))
write("icon-192-v1.png", png(192, True))
write("icon-512-v1.png", png(512, True))
write("icon-maskable-512-v1.png", png(512, False))
sizes = [16, 32, 48]
pngs = [png(s, True) for s in sizes]
off = 6 + 16 * len(sizes)
ent = b""
for s, p in zip(sizes, pngs):
    ent += struct.pack("<BBBBHHII", s, s, 0, 0, 1, 32, len(p), off)
    off += len(p)
write("favicon-v1.ico", struct.pack("<HHH", 0, 1, len(sizes)) + ent + b"".join(pngs))
