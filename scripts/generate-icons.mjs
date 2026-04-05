import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { deflateSync } from "node:zlib";

const __dirname = dirname(fileURLToPath(import.meta.url));
const rootDir = dirname(__dirname);
const iconsDir = join(rootDir, "src-tauri", "icons");

const size = 1024;
const buffer = Buffer.alloc(size * size * 4);

function setPixel(x, y, r, g, b, a = 255) {
  const index = (y * size + x) * 4;
  buffer[index] = r;
  buffer[index + 1] = g;
  buffer[index + 2] = b;
  buffer[index + 3] = a;
}

function clamp(value, min, max) {
  return Math.max(min, Math.min(max, value));
}

function smoothstep(edge0, edge1, value) {
  const x = clamp((value - edge0) / (edge1 - edge0), 0, 1);
  return x * x * (3 - 2 * x);
}

function mix(a, b, t) {
  return Math.round(a + (b - a) * t);
}

function roundedRectAlpha(x, y, rectX, rectY, rectW, rectH, radius) {
  const dx = Math.max(rectX - x, 0, x - (rectX + rectW));
  const dy = Math.max(rectY - y, 0, y - (rectY + rectH));
  const dist = Math.max(
    Math.hypot(Math.max(dx - radius, 0), Math.max(dy - radius, 0)),
    Math.max(dx, dy) - radius
  );
  return 1 - smoothstep(0, 2.2, dist);
}

function circleAlpha(x, y, cx, cy, radius, feather = 2) {
  const dist = Math.hypot(x - cx, y - cy);
  return 1 - smoothstep(radius - feather, radius + feather, dist);
}

for (let y = 0; y < size; y += 1) {
  for (let x = 0; x < size; x += 1) {
    const nx = x / (size - 1);
    const ny = y / (size - 1);

    const bgMix = smoothstep(0, 1, ny * 0.78 + nx * 0.22);
    const r = mix(20, 8, bgMix);
    const g = mix(132, 74, bgMix);
    const b = mix(255, 181, bgMix);

    const panelAlpha = roundedRectAlpha(x, y, 124, 124, 776, 776, 220);
    const shine = circleAlpha(x, y, 338, 304, 222, 90) * 0.22;
    const shadow = circleAlpha(x, y, 724, 776, 270, 120) * 0.18;

    const pr = mix(r, 245, panelAlpha * 0.1 + shine);
    const pg = mix(g, 250, panelAlpha * 0.1 + shine);
    const pb = mix(b, 255, panelAlpha * 0.08 + shine);

    let finalR = mix(232, pr, 0.96);
    let finalG = mix(238, pg, 0.96);
    let finalB = mix(248, pb, 0.96);

    finalR = mix(finalR, 0, shadow * 0.16);
    finalG = mix(finalG, 22, shadow * 0.16);
    finalB = mix(finalB, 64, shadow * 0.16);

    const alpha = Math.round(panelAlpha * 255);
    setPixel(x, y, finalR, finalG, finalB, alpha);
  }
}

function fillRoundedRect(rectX, rectY, rectW, rectH, radius, color) {
  for (let y = rectY; y < rectY + rectH; y += 1) {
    for (let x = rectX; x < rectX + rectW; x += 1) {
      const alpha = roundedRectAlpha(x, y, rectX, rectY, rectW, rectH, radius);
      if (alpha <= 0) continue;
      setPixel(
        x,
        y,
        color[0],
        color[1],
        color[2],
        Math.round(color[3] * alpha)
      );
    }
  }
}

fillRoundedRect(272, 258, 480, 108, 54, [255, 255, 255, 240]);
fillRoundedRect(272, 458, 480, 108, 54, [255, 255, 255, 218]);
fillRoundedRect(272, 658, 310, 108, 54, [255, 255, 255, 194]);

for (let y = 0; y < size; y += 1) {
  for (let x = 0; x < size; x += 1) {
    const glow = circleAlpha(x, y, 792, 252, 124, 64);
    if (glow <= 0) continue;

    const index = (y * size + x) * 4;
    buffer[index] = mix(buffer[index], 255, glow * 0.5);
    buffer[index + 1] = mix(buffer[index + 1], 255, glow * 0.38);
    buffer[index + 2] = mix(buffer[index + 2], 255, glow * 0.22);
    buffer[index + 3] = Math.max(buffer[index + 3], Math.round(255 * glow * 0.65));
  }
}

function crc32(input) {
  let crc = 0xffffffff;
  for (let i = 0; i < input.length; i += 1) {
    crc ^= input[i];
    for (let j = 0; j < 8; j += 1) {
      const mask = -(crc & 1);
      crc = (crc >>> 1) ^ (0xedb88320 & mask);
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function pngChunk(type, data) {
  const typeBuffer = Buffer.from(type, "ascii");
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length, 0);

  const crcBuffer = Buffer.alloc(4);
  crcBuffer.writeUInt32BE(crc32(Buffer.concat([typeBuffer, data])), 0);

  return Buffer.concat([length, typeBuffer, data, crcBuffer]);
}

const raw = Buffer.alloc((size * 4 + 1) * size);
for (let y = 0; y < size; y += 1) {
  const rowStart = y * (size * 4 + 1);
  raw[rowStart] = 0;
  buffer.copy(raw, rowStart + 1, y * size * 4, (y + 1) * size * 4);
}

const png = Buffer.concat([
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
  pngChunk(
    "IHDR",
    Buffer.from([
      0, 0, 4, 0,
      0, 0, 4, 0,
      8,
      6,
      0,
      0,
      0,
    ])
  ),
  pngChunk("IDAT", deflateSync(raw, { level: 9 })),
  pngChunk("IEND", Buffer.alloc(0)),
]);

mkdirSync(iconsDir, { recursive: true });
rmSync(join(iconsDir, "iconset"), { recursive: true, force: true });
writeFileSync(join(iconsDir, "icon.png"), png);

console.log("Generated src-tauri/icons/icon.png");
