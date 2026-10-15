#!/usr/bin/env python3
"""Convert PPM (P6) images to PNG format.

Usage:
    python3 scripts/ppm2png.py [file_or_glob_pattern]
    # Default pattern: /tmp/snap-*.ppm
"""

import glob
import os
import struct
import sys
import zlib


def parse_ppm(data: bytes):
    """Parse binary PPM (P6) data."""
    offset = 0
    n = len(data)

    def next_token() -> bytes:
        nonlocal offset
        while offset < n:
            b = data[offset]
            if b in (0x20, 0x09, 0x0A, 0x0D):  # space, tab, LF, CR
                offset += 1
                continue
            if b == 0x23:  # '#' comment
                while offset < n and data[offset] not in (0x0A, 0x0D):
                    offset += 1
                continue
            break

        if offset >= n:
            return None

        start = offset
        while offset < n:
            b = data[offset]
            if b in (0x20, 0x09, 0x0A, 0x0D, 0x23):
                break
            offset += 1
        return data[start:offset]

    magic = next_token()
    if magic != b"P6":
        return None

    w_tok = next_token()
    h_tok = next_token()
    max_tok = next_token()
    if not (w_tok and h_tok and max_tok):
        return None

    try:
        width = int(w_tok.decode("ascii"))
        height = int(h_tok.decode("ascii"))
        maxval = int(max_tok.decode("ascii"))
    except (ValueError, UnicodeDecodeError):
        return None

    if width <= 0 or height <= 0 or maxval != 255:
        return None

    # Single whitespace byte after maxval before binary raster
    if offset < n and data[offset] in (0x20, 0x09, 0x0A, 0x0D):
        offset += 1

    raw = data[offset:]
    expected_size = width * height * 3
    if len(raw) < expected_size:
        return None

    return width, height, raw[:expected_size]


def make_chunk(chunk_type: bytes, data: bytes) -> bytes:
    """Create a PNG chunk with length and CRC32."""
    length = len(data)
    payload = chunk_type + data
    crc = zlib.crc32(payload) & 0xFFFFFFFF
    return struct.pack(">I", length) + payload + struct.pack(">I", crc)


def ppm_to_png(ppm_path: str, png_path: str) -> bool:
    """Convert a single PPM file to PNG format."""
    try:
        with open(ppm_path, "rb") as f:
            data = f.read()
    except OSError as e:
        print(f"Error reading {ppm_path}: {e}", file=sys.stderr)
        return False

    parsed = parse_ppm(data)
    if not parsed:
        print(f"Failed to parse PPM file: {ppm_path}", file=sys.stderr)
        return False

    width, height, raw = parsed
    stride = width * 3

    # Add PNG filter byte (0: None) before each row
    scanlines = bytearray(height * (stride + 1))
    for y in range(height):
        row_start_src = y * stride
        row_start_dst = y * (stride + 1)
        scanlines[row_start_dst] = 0  # Filter type 0: None
        scanlines[row_start_dst + 1 : row_start_dst + 1 + stride] = raw[
            row_start_src : row_start_src + stride
        ]

    compressed = zlib.compress(scanlines)

    # PNG Signature: \x89PNG\r\n\x1a\n
    png_signature = b"\x89PNG\r\n\x1a\n"

    # IHDR: width(4), height(4), bit_depth(1), color_type(1), compression(1), filter(1), interlace(1)
    ihdr_data = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    ihdr_chunk = make_chunk(b"IHDR", ihdr_data)
    idat_chunk = make_chunk(b"IDAT", compressed)
    iend_chunk = make_chunk(b"IEND", b"")

    try:
        with open(png_path, "wb") as f:
            f.write(png_signature)
            f.write(ihdr_chunk)
            f.write(idat_chunk)
            f.write(iend_chunk)
        return True
    except OSError as e:
        print(f"Error writing {png_path}: {e}", file=sys.stderr)
        return False


def main():
    target = sys.argv[1] if len(sys.argv) > 1 else "/tmp/snap-*.ppm"

    if os.path.isfile(target):
        out = target[:-4] + ".png" if target.endswith(".ppm") else target + ".png"
        if ppm_to_png(target, out):
            print("Converted:", out)
        return 0

    matched = glob.glob(target)
    if not matched:
        print(f"No files matched pattern: {target}", file=sys.stderr)
        return 0

    for path in sorted(matched):
        out = path[:-4] + ".png" if path.endswith(".ppm") else path + ".png"
        if ppm_to_png(path, out):
            print("Converted:", out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
