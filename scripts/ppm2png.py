#!/usr/bin/env python3
import struct
import zlib
import glob
import sys

def ppm_to_png(ppm_path, png_path):
    with open(ppm_path, "rb") as f:
        header = f.readline().decode().strip()
        if header != "P6":
            return
        dims = f.readline().decode().strip().split()
        while len(dims) == 0 or dims[0].startswith("#"):
            dims = f.readline().decode().strip().split()
        w, h = int(dims[0]), int(dims[1])
        maxval = f.readline().decode().strip()
        raw = f.read()

    stride = w * 3
    scanlines = bytearray()
    for y in range(h):
        scanlines.append(0)
        scanlines.extend(raw[y * stride : (y + 1) * stride])

    def chunk(tag, data):
        c = tag + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c) & 0xffffffff)

    png = bytearray(b"\x89PNG\r\n\x1a\n")
    png.extend(chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)))
    png.extend(chunk(b"IDAT", zlib.compress(bytes(scanlines))))
    png.extend(chunk(b"IEND", b""))

    with open(png_path, "wb") as f:
        f.write(png)

if __name__ == "__main__":
    pattern = sys.argv[1] if len(sys.argv) > 1 else "/tmp/snap-*.ppm"
    for p in glob.glob(pattern):
        out = p[:-4] + ".png"
        ppm_to_png(p, out)
        print("Converted:", out)
