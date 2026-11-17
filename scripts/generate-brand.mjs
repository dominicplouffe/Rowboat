// Regenerates every brand asset from the source SVGs in assets/brand/.
//
//   npm run icons
//
// Sources (edit these):   assets/brand/icon.svg   app icon, 1024×1024 master
//                         assets/brand/logo.svg   the mark without the tile
//                         assets/brand/glyph.svg  hull-only glyph for 16–32 px
// Everything else in assets/brand/, src-tauri/icons/ and src-tauri/installer/
// is generated and committed, so normal builds need none of this.

import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Resvg } from '@resvg/resvg-js';
import opentype from 'opentype.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const brand = join(root, 'assets/brand');
const icons = join(root, 'src-tauri/icons');
const installer = join(root, 'src-tauri/installer');
mkdirSync(installer, { recursive: true });

const NAVY = '#0E2A47';
const CREAM = '#FFF4E0';
const BLUE = '#2D9CDB';

const read = (name) => readFileSync(join(brand, name), 'utf8');
const inner = (svg) => svg.replace(/^[\s\S]*?<svg[^>]*>/, '').replace(/<\/svg>\s*$/, '');

function render(svg, width) {
  return new Resvg(svg, { fitTo: { mode: 'width', value: width } }).render();
}
const png = (svg, width) => render(svg, width).asPng();

// ---- 1. Master PNG and the platform icons `tauri icon` derives from it ----

const iconSvg = read('icon.svg');
const glyphSvg = read('glyph.svg');
const logoSvg = read('logo.svg');

writeFileSync(join(brand, 'icon-1024.png'), png(iconSvg, 1024));
execFileSync(
  join(root, 'node_modules/.bin/tauri'),
  ['icon', join(brand, 'icon-1024.png'), '--output', icons],
  { cwd: root, stdio: 'inherit' },
);
// Desktop only: drop the mobile icon sets.
for (const dir of ['ios', 'android']) rmSync(join(icons, dir), { recursive: true, force: true });

// ---- 2. Small sizes use the simplified glyph (oars and wave drop away) ----

for (const size of [16, 32]) writeFileSync(join(brand, `glyph-${size}.png`), png(glyphSvg, size));
writeFileSync(join(icons, '32x32.png'), png(glyphSvg, 32));
// Linux hicolor sizes that `tauri icon` does not emit under these names.
for (const size of [256, 512])
  writeFileSync(join(icons, `${size}x${size}.png`), png(iconSvg, size));
// Linux hicolor sizes that `tauri icon` does not emit under these names.
for (const size of [256, 512])
  writeFileSync(join(icons, `${size}x${size}.png`), png(iconSvg, size));

const iconAt = (size) => (size <= 32 ? png(glyphSvg, size) : png(iconSvg, size));

// ICO: a directory of PNG-compressed images.
function ico(sizes) {
  const images = sizes.map((size) => ({ size, data: iconAt(size) }));
  const header = Buffer.alloc(6);
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(images.length, 4);
  let offset = 6 + 16 * images.length;
  const entries = images.map(({ size, data }) => {
    const e = Buffer.alloc(16);
    e.writeUInt8(size === 256 ? 0 : size, 0);
    e.writeUInt8(size === 256 ? 0 : size, 1);
    e.writeUInt16LE(1, 4); // planes
    e.writeUInt16LE(32, 6); // bits per pixel
    e.writeUInt32LE(data.length, 8);
    e.writeUInt32LE(offset, 12);
    offset += data.length;
    return e;
  });
  return Buffer.concat([header, ...entries, ...images.map((i) => i.data)]);
}
writeFileSync(join(icons, 'icon.ico'), ico([16, 24, 32, 48, 64, 256]));

// ICNS: typed chunks of PNG data, @1x and @2x for 16–512 pt.
function icns() {
  const types = [
    ['icp4', 16],
    ['icp5', 32],
    ['icp6', 64],
    ['ic07', 128],
    ['ic08', 256],
    ['ic09', 512],
    ['ic10', 1024],
    ['ic11', 32],
    ['ic12', 64],
    ['ic13', 256],
    ['ic14', 512],
  ];
  const chunks = types.map(([type, size]) => {
    const data = iconAt(size);
    const head = Buffer.alloc(8);
    head.write(type, 0, 'ascii');
    head.writeUInt32BE(data.length + 8, 4);
    return Buffer.concat([head, data]);
  });
  const body = Buffer.concat(chunks);
  const head = Buffer.alloc(8);
  head.write('icns', 0, 'ascii');
  head.writeUInt32BE(body.length + 8, 4);
  return Buffer.concat([head, body]);
}
writeFileSync(join(icons, 'icon.icns'), icns());

// ---- 3. Wordmark: "Row" bold + "boat" regular, outlined from Nunito (OFL) ----

const font = (weight) => {
  const file = join(
    root,
    `node_modules/@fontsource/nunito/files/nunito-latin-${weight}-normal.woff`,
  );
  const bytes = readFileSync(file);
  return opentype.parse(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength));
};
const bold = font(800);
const regular = font(400);

/**
 * Lays text out glyph by glyph (advance widths + pair kerning) and returns its
 * outline. Done by hand because opentype.js cannot apply some of Nunito's
 * contextual substitutions, and plain Latin text needs none of them.
 */
function outline(face, text, size, startX = 0) {
  const scale = size / face.unitsPerEm;
  let x = startX;
  let d = '';
  let previous = null;
  for (const char of text) {
    const glyph = face.charToGlyph(char);
    if (previous) x += face.getKerningValue(previous, glyph) * scale;
    d += glyph.getPath(x, 0, size).toPathData(2);
    x += glyph.advanceWidth * scale;
    previous = glyph;
  }
  return { d, width: x - startX };
}

/** "Row" bold + "boat" regular as one outlined path. */
function wordmark(size) {
  const row = outline(bold, 'Row', size);
  const boat = outline(regular, 'boat', size, row.width);
  return { d: row.d + boat.d, width: row.width + boat.width };
}

const logoBox = { x: 140, y: 286, w: 776, h: 568 };

for (const [theme, ink] of [
  ['light', NAVY],
  ['dark', CREAM],
]) {
  const size = 96;
  const text = wordmark(size);
  const pad = 8;
  const w = Math.ceil(text.width + pad * 2);
  const h = 124;
  const wordmarkSvg =
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${w} ${h}" width="${w}" height="${h}">` +
    `<title>Rowboat</title><path transform="translate(${pad} 92)" fill="${ink}" d="${text.d}"/></svg>\n`;
  writeFileSync(join(brand, `wordmark-${theme}.svg`), wordmarkSvg);

  // Lockup: mark on the left, wordmark on the right.
  const markH = 124;
  const markW = (logoBox.w / logoBox.h) * markH;
  const lockW = Math.ceil(markW + 28 + text.width + pad);
  const mark = theme === 'dark' ? inner(logoSvg).replaceAll(NAVY, CREAM) : inner(logoSvg);
  const lockupSvg =
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${lockW} ${markH}" width="${lockW}" height="${markH}">` +
    `<title>Rowboat</title>` +
    `<svg x="0" y="0" width="${markW.toFixed(1)}" height="${markH}" viewBox="${logoBox.x} ${logoBox.y} ${logoBox.w} ${logoBox.h}">${mark}</svg>` +
    `<path transform="translate(${(markW + 28).toFixed(1)} 94)" fill="${ink}" d="${text.d}"/></svg>\n`;
  writeFileSync(join(brand, `logo-wordmark-${theme}.svg`), lockupSvg);
  writeFileSync(join(brand, `logo-wordmark-${theme}.png`), png(lockupSvg, lockW * 2));
}
writeFileSync(join(brand, 'logo.png'), png(logoSvg, 776));

// ---- 4. GitHub social preview, 1280×640 ----

{
  const title = wordmark(150);
  const tagline = outline(
    regular,
    'A fast SQL client with an AI copilot that knows your schema.',
    34,
  );
  const markH = 250;
  const markW = (logoBox.w / logoBox.h) * markH;
  const social =
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1280 640" width="1280" height="640">` +
    `<defs><linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#F4FAFE"/><stop offset="1" stop-color="#D5ECFA"/></linearGradient></defs>` +
    `<rect width="1280" height="640" fill="url(#bg)"/>` +
    `<svg x="${((1280 - markW) / 2).toFixed(1)}" y="70" width="${markW.toFixed(1)}" height="${markH}" viewBox="${logoBox.x} ${logoBox.y} ${logoBox.w} ${logoBox.h}">${inner(logoSvg)}</svg>` +
    `<path transform="translate(${((1280 - title.width) / 2).toFixed(1)} 470)" fill="${NAVY}" d="${title.d}"/>` +
    `<path transform="translate(${((1280 - tagline.width) / 2).toFixed(1)} 548)" fill="${NAVY}" fill-opacity="0.72" d="${tagline.d}"/>` +
    `</svg>`;
  writeFileSync(join(brand, 'social-preview.png'), png(social, 1280));
}

// ---- 5. Installer art ----

// DMG background 660×400 (+@2x): a calm tile with an arrow from app to Applications.
{
  const dmg =
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 660 400" width="660" height="400">` +
    `<defs><linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#F4FAFE"/><stop offset="1" stop-color="#D5ECFA"/></linearGradient></defs>` +
    `<rect width="660" height="400" fill="url(#bg)"/>` +
    `<path d="M0 352C55 328 110 328 165 352S275 376 330 352S440 328 495 352S605 376 660 352V400H0Z" fill="${BLUE}" fill-opacity="0.22"/>` +
    `<path d="M282 220H366M346 200L368 220L346 240" fill="none" stroke="${NAVY}" stroke-opacity="0.55" stroke-width="7" stroke-linecap="round" stroke-linejoin="round"/>` +
    `</svg>`;
  writeFileSync(join(installer, 'dmg-background.png'), png(dmg, 660));
  writeFileSync(join(installer, 'dmg-background@2x.png'), png(dmg, 1320));
}

// NSIS wants 24-bit BMPs: header 150×57, sidebar 164×314.
function bmp(svg, width, height) {
  const { pixels } = render(svg, width); // RGBA, top-down
  const rowSize = Math.ceil((width * 3) / 4) * 4;
  const out = Buffer.alloc(54 + rowSize * height);
  out.write('BM', 0, 'ascii');
  out.writeUInt32LE(out.length, 2);
  out.writeUInt32LE(54, 10);
  out.writeUInt32LE(40, 14);
  out.writeInt32LE(width, 18);
  out.writeInt32LE(height, 22); // positive: rows stored bottom-up
  out.writeUInt16LE(1, 26);
  out.writeUInt16LE(24, 28);
  out.writeUInt32LE(rowSize * height, 34);
  out.writeInt32LE(2835, 38);
  out.writeInt32LE(2835, 42);
  for (let y = 0; y < height; y++) {
    const row = 54 + (height - 1 - y) * rowSize;
    for (let x = 0; x < width; x++) {
      const p = (y * width + x) * 4;
      const a = pixels[p + 3] / 255;
      // Flatten onto white; BMP stores BGR.
      out[row + x * 3] = Math.round(pixels[p + 2] * a + 255 * (1 - a));
      out[row + x * 3 + 1] = Math.round(pixels[p + 1] * a + 255 * (1 - a));
      out[row + x * 3 + 2] = Math.round(pixels[p] * a + 255 * (1 - a));
    }
  }
  return out;
}
{
  const header =
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 150 57" width="150" height="57">` +
    `<rect width="150" height="57" fill="#FFFFFF"/>` +
    `<svg x="84" y="8" width="61" height="41" viewBox="${logoBox.x} ${logoBox.y} ${logoBox.w} ${logoBox.h}">${inner(logoSvg)}</svg></svg>`;
  writeFileSync(join(installer, 'nsis-header.bmp'), bmp(header, 150, 57));

  const side = wordmark(30);
  const sidebar =
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 164 314" width="164" height="314">` +
    `<defs><linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#F4FAFE"/><stop offset="1" stop-color="#D5ECFA"/></linearGradient></defs>` +
    `<rect width="164" height="314" fill="url(#bg)"/>` +
    `<svg x="22" y="78" width="120" height="80" viewBox="${logoBox.x} ${logoBox.y} ${logoBox.w} ${logoBox.h}">${inner(logoSvg)}</svg>` +
    `<path transform="translate(${((164 - side.width) / 2).toFixed(1)} 206)" fill="${NAVY}" d="${side.d}"/>` +
    `<path d="M0 286C27 274 55 274 82 286S137 298 164 286V314H0Z" fill="${BLUE}" fill-opacity="0.3"/>` +
    `</svg>`;
  writeFileSync(join(installer, 'nsis-sidebar.bmp'), bmp(sidebar, 164, 314));
}

console.log('Brand assets regenerated.');
