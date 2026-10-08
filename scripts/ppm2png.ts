#!/usr/bin/env -S deno run --allow-read --allow-write

// CRC32 table for IEEE 802.3
const crcTable = new Uint32Array(256);
for (let n = 0; n < 256; n++) {
  let c = n;
  for (let k = 0; k < 8; k++) {
    c = (c & 1) ? (0xedb88320 ^ (c >>> 1)) : (c >>> 1);
  }
  crcTable[n] = c;
}

function crc32(buf: Uint8Array): number {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) {
    c = crcTable[(c ^ buf[i]) & 0xff] ^ (c >>> 1);
  }
  return (c ^ 0xffffffff) >>> 0;
}

function makeChunk(typeStr: string, data: Uint8Array): Uint8Array {
  const typeBytes = new TextEncoder().encode(typeStr);
  const totalPayload = new Uint8Array(4 + data.length);
  totalPayload.set(typeBytes, 0);
  totalPayload.set(data, 4);

  const crc = crc32(totalPayload);

  const out = new Uint8Array(4 + 4 + data.length + 4);
  const view = new DataView(out.buffer, out.byteOffset, out.byteLength);

  view.setUint32(0, data.length, false);
  out.set(totalPayload, 4);
  view.setUint32(8 + data.length, crc, false);

  return out;
}

function parsePpm(bytes: Uint8Array): { width: number; height: number; raw: Uint8Array } | null {
  let offset = 0;

  function nextToken(): string | null {
    while (offset < bytes.length) {
      const b = bytes[offset];
      if (b === 0x20 || b === 0x09 || b === 0x0a || b === 0x0d) {
        offset++;
        continue;
      }
      if (b === 0x23) {
        // Comment until newline
        while (offset < bytes.length && bytes[offset] !== 0x0a && bytes[offset] !== 0x0d) {
          offset++;
        }
        continue;
      }
      break;
    }
    if (offset >= bytes.length) return null;

    const start = offset;
    while (offset < bytes.length) {
      const b = bytes[offset];
      if (b === 0x20 || b === 0x09 || b === 0x0a || b === 0x0d || b === 0x23) {
        break;
      }
      offset++;
    }
    return new TextDecoder().decode(bytes.subarray(start, offset));
  }

  const magic = nextToken();
  if (magic !== "P6") return null;

  const wStr = nextToken();
  const hStr = nextToken();
  const maxStr = nextToken();
  if (!wStr || !hStr || !maxStr) return null;

  const width = parseInt(wStr, 10);
  const height = parseInt(hStr, 10);
  const maxval = parseInt(maxStr, 10);
  if (isNaN(width) || isNaN(height) || isNaN(maxval)) return null;

  // Single whitespace char after maxval before binary raster
  if (offset < bytes.length) {
    const b = bytes[offset];
    if (b === 0x20 || b === 0x09 || b === 0x0a || b === 0x0d) {
      offset++;
    }
  }

  const raw = bytes.subarray(offset);
  return { width, height, raw };
}

async function ppmToPng(ppmPath: string, pngPath: string): Promise<boolean> {
  const bytes = await Deno.readFile(ppmPath);
  const parsed = parsePpm(bytes);
  if (!parsed) {
    console.error(`Failed to parse PPM file: ${ppmPath}`);
    return false;
  }

  const { width, height, raw } = parsed;
  const stride = width * 3;
  const scanlines = new Uint8Array(height * (stride + 1));

  for (let y = 0; y < height; y++) {
    const rowStart = y * (stride + 1);
    scanlines[rowStart] = 0; // Filter method 0: None
    scanlines.set(raw.subarray(y * stride, (y + 1) * stride), rowStart + 1);
  }

  const cs = new CompressionStream("deflate");
  const writer = cs.writable.getWriter();
  writer.write(scanlines);
  writer.close();
  const compressedBuffer = await new Response(cs.readable).arrayBuffer();
  const compressed = new Uint8Array(compressedBuffer);

  // IHDR: 13 bytes
  const ihdrData = new Uint8Array(13);
  const ihdrView = new DataView(ihdrData.buffer);
  ihdrView.setUint32(0, width, false);
  ihdrView.setUint32(4, height, false);
  ihdrData[8] = 8; // 8 bits per sample
  ihdrData[9] = 2; // Color type: RGB
  ihdrData[10] = 0; // Compression: Deflate
  ihdrData[11] = 0; // Filter method 0
  ihdrData[12] = 0; // Interlace: None

  const signature = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const ihdrChunk = makeChunk("IHDR", ihdrData);
  const idatChunk = makeChunk("IDAT", compressed);
  const iendChunk = makeChunk("IEND", new Uint8Array(0));

  const totalLength = signature.length + ihdrChunk.length + idatChunk.length + iendChunk.length;
  const png = new Uint8Array(totalLength);
  let pos = 0;
  png.set(signature, pos);
  pos += signature.length;
  png.set(ihdrChunk, pos);
  pos += ihdrChunk.length;
  png.set(idatChunk, pos);
  pos += idatChunk.length;
  png.set(iendChunk, pos);

  await Deno.writeFile(pngPath, png);
  return true;
}

const target = Deno.args[0] ?? "/tmp/snap-*.ppm";

try {
  const stat = await Deno.stat(target);
  if (stat.isFile) {
    const out = target.replace(/\.ppm$/, "") + ".png";
    if (await ppmToPng(target, out)) {
      console.log("Converted:", out);
    }
    Deno.exit(0);
  }
} catch {
  // Not a single file, handle directory search
}

const dir = target.includes("/") ? target.substring(0, target.lastIndexOf("/")) : ".";
const filePattern = target.includes("/") ? target.substring(target.lastIndexOf("/") + 1) : target;
const regex = new RegExp("^" + filePattern.replace(/\./g, "\\.").replace(/\*/g, ".*") + "$");

try {
  for await (const entry of Deno.readDir(dir)) {
    if (entry.isFile && regex.test(entry.name)) {
      const fullPath = `${dir}/${entry.name}`;
      const out = fullPath.replace(/\.ppm$/, "") + ".png";
      if (await ppmToPng(fullPath, out)) {
        console.log("Converted:", out);
      }
    }
  }
} catch (e) {
  console.error("Error searching pattern:", e);
}
