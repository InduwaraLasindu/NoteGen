// Deterministic TrueType (.ttf) binary font generator + SVG contour tracer for NoteGen Writer Profiles.
// Ported from rust/src/fontpack.rs so users can preview and export real installable .ttf fonts directly in the browser and tests.

import { WriterProfile, averageGlyphGrid } from './engine';

export interface FontPackOptions {
  family: string;
  spacing: number;   // 0..500
  threshold: number; // 32..224
}

export interface TracedGlyph {
  char: string;
  contours: Array<Array<[number, number]>>;
  advance: number;
  svgPath: string;
  averagedGrid: number[];
}

export interface GeneratedFontPack {
  family: string;
  glyphCount: number;
  coveredChars: string[];
  missingChars: string[];
  ttfBytes: Uint8Array;
  glyphs: TracedGlyph[];
  readmeText: string;
}

export function traceGridContours(
  inkGrid: number[],
  threshold: number,
  ch: string,
  spacing: number
): { contours: Array<Array<[number, number]>>; advance: number; svgPath: string } {
  const W = 16;
  const H = 20;
  const isInk = (x: number, y: number): boolean =>
    x >= 0 && y >= 0 && x < W && y < H && inkGrid[y * W + x] >= threshold;

  // Find bounding box of active ink cells
  let minX = W;
  let maxX = 0;
  let minY = H;
  let maxY = 0;
  const boxes: Array<[number, number]> = [];

  for (let y = 0; y < H; y++) {
    for (let x = 0; x < W; x++) {
      if (isInk(x, y)) {
        minX = Math.min(minX, x);
        maxX = Math.max(maxX, x + 1);
        minY = Math.min(minY, y);
        maxY = Math.max(maxY, y + 1);
        boxes.push([x, y]);
      }
    }
  }

  if (boxes.length === 0) {
    throw new Error(
      `NG620: Averaging removed all ink for ${ch}. Add consistent samples or lower the ink threshold.`
    );
  }

  const descender = 'gjpqy,;'.includes(ch);
  const targetHeight = /[.,;:'"-]/.test(ch)
    ? 220
    : ch >= 'a' && ch <= 'z'
    ? descender
      ? 720
      : 540
    : 780;

  const spanH = Math.max(1, maxY - minY);
  const spanW = Math.max(1, maxX - minX);
  const scale = Math.min(targetHeight / spanH, 1400 / spanW);
  const baseOffset = descender ? -180 : 0;

  // Merge horizontal runs per row into clean clockwise TrueType contour rectangles
  const contours: Array<Array<[number, number]>> = [];
  const svgParts: string[] = [];

  for (let y = minY; y < maxY; y++) {
    let x = minX;
    while (x < maxX) {
      if (!isInk(x, y)) {
        x++;
        continue;
      }
      const runStart = x;
      while (x < maxX && isInk(x, y)) {
        x++;
      }
      const runEnd = x;

      const left = Math.round((runStart - minX) * scale + 45);
      const right = Math.round((runEnd - minX) * scale + 45);
      const bottom = Math.round((maxY - (y + 1)) * scale + baseOffset);
      const top = Math.round((maxY - y) * scale + baseOffset);

      contours.push([
        [left, bottom],
        [left, top],
        [right, top],
        [right, bottom],
      ]);

      const svgYTop = 800 - top;
      const svgYBot = 800 - bottom;
      svgParts.push(
        `M ${left},${svgYTop} L ${right},${svgYTop} L ${right},${svgYBot} L ${left},${svgYBot} Z`
      );
    }
  }

  const advance = Math.max(
    180,
    Math.min(1800, Math.round(spanW * scale + 50 + spacing))
  );

  return {
    contours,
    advance,
    svgPath: svgParts.join(' '),
  };
}

class ByteWriter {
  private bytes: number[] = [];

  u8(v: number) {
    this.bytes.push(v & 0xff);
  }
  u16(v: number) {
    this.bytes.push((v >>> 8) & 0xff, v & 0xff);
  }
  i16(v: number) {
    const u = v < 0 ? 0x10000 + v : v;
    this.u16(u);
  }
  u32(v: number) {
    this.bytes.push(
      (v >>> 24) & 0xff,
      (v >>> 16) & 0xff,
      (v >>> 8) & 0xff,
      v & 0xff
    );
  }
  extend(arr: number[] | Uint8Array) {
    for (let i = 0; i < arr.length; i++) {
      this.bytes.push(arr[i] & 0xff);
    }
  }
  pad4() {
    while (this.bytes.length % 4 !== 0) {
      this.bytes.push(0);
    }
  }
  toUint8Array(): Uint8Array {
    return new Uint8Array(this.bytes);
  }
  get length(): number {
    return this.bytes.length;
  }
}

export function computeTtfTableChecksum(bytes: Uint8Array): number {
  let sum = 0;
  for (let i = 0; i < bytes.length; i += 4) {
    const b0 = bytes[i] ?? 0;
    const b1 = bytes[i + 1] ?? 0;
    const b2 = bytes[i + 2] ?? 0;
    const b3 = bytes[i + 3] ?? 0;
    const word = ((b0 << 24) | (b1 << 16) | (b2 << 8) | b3) >>> 0;
    sum = (sum + word) >>> 0;
  }
  return sum >>> 0;
}

function encodeSimpleGlyph(contours: Array<Array<[number, number]>>): Uint8Array {
  if (contours.length === 0) return new Uint8Array(0);
  const points = contours.flat();
  const w = new ByteWriter();
  w.i16(contours.length);

  let minX = points[0][0];
  let minY = points[0][1];
  let maxX = points[0][0];
  let maxY = points[0][1];
  for (const [px, py] of points) {
    if (px < minX) minX = px;
    if (py < minY) minY = py;
    if (px > maxX) maxX = px;
    if (py > maxY) maxY = py;
  }
  w.i16(minX);
  w.i16(minY);
  w.i16(maxX);
  w.i16(maxY);

  let end = 0;
  for (const c of contours) {
    end += c.length;
    w.u16(end - 1);
  }
  w.u16(0); // instructionLength = 0

  for (let i = 0; i < points.length; i++) {
    w.u8(1); // on-curve point flag
  }

  let lastX = 0;
  for (const [px] of points) {
    w.i16(px - lastX);
    lastX = px;
  }
  let lastY = 0;
  for (const [, py] of points) {
    w.i16(py - lastY);
    lastY = py;
  }
  w.pad4();
  return w.toUint8Array();
}

export function buildTrueTypeFont(
  glyphs: Array<{ char: string; contours: Array<Array<[number, number]>>; advance: number }>,
  family: string
): Uint8Array {
  const sorted = [...glyphs].sort((a, b) => a.char.charCodeAt(0) - b.char.charCodeAt(0));

  const fallback = {
    contours: [
      [
        [50, 0],
        [50, 700],
        [450, 700],
        [450, 0],
      ] as Array<[number, number]>,
    ],
    advance: 500,
  };
  const spaceGlyph = {
    contours: [] as Array<Array<[number, number]>>,
    advance: 300,
  };

  const allGlyphs = [fallback, spaceGlyph, ...sorted];

  const glyfWriter = new ByteWriter();
  const locaWriter = new ByteWriter();
  const hmtxWriter = new ByteWriter();

  for (const g of allGlyphs) {
    locaWriter.u32(glyfWriter.length);
    glyfWriter.extend(encodeSimpleGlyph(g.contours));
    hmtxWriter.u16(g.advance);
    const lsb =
      g.contours.length === 0
        ? 0
        : Math.min(...g.contours.flat().map((p) => p[0]));
    hmtxWriter.i16(lsb);
  }
  locaWriter.u32(glyfWriter.length);

  // head table (54 bytes)
  const head = new ByteWriter();
  head.u32(0x00010000);
  head.u32(0x00010000);
  head.u32(0); // checkSumAdjustment placeholder
  head.u32(0x5f0f3cf5); // magicNumber
  head.u16(3);
  head.u16(1000); // unitsPerEm
  head.u32(0);
  head.u32(3850070400 >>> 0);
  head.u32(0);
  head.u32(3850070400 >>> 0);
  for (const n of [0, -200, 2000, 800, 0, 8, 2, 1, 0]) {
    head.i16(n);
  }

  // hhea table (36 bytes)
  const hhea = new ByteWriter();
  hhea.u32(0x00010000);
  for (const n of [850, -250, 100]) hhea.i16(n);
  hhea.u16(Math.max(...allGlyphs.map((g) => g.advance), 500));
  for (const n of [0, 0, 2000, 1, 0, 0, 0, 0, 0, 0, 0]) hhea.i16(n);
  hhea.u16(allGlyphs.length);

  // maxp table (32 bytes)
  const maxp = new ByteWriter();
  maxp.u32(0x00010000);
  maxp.u16(allGlyphs.length);
  maxp.u16(
    Math.max(
      ...allGlyphs.map((g) =>
        g.contours.reduce((acc, c) => acc + c.length, 0)
      ),
      0
    )
  );
  maxp.u16(Math.max(...allGlyphs.map((g) => g.contours.length), 0));
  for (const n of [0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0]) maxp.u16(n);

  // cmap table (format 4)
  const mappings: Array<[number, number]> = [[32, 1]];
  sorted.forEach((g, idx) => {
    mappings.push([g.char.charCodeAt(0), idx + 2]);
  });
  mappings.push([0xffff, 0]);
  const segCount = mappings.length;
  const pow2 = 1 << Math.floor(Math.log2(segCount));
  const entrySelector = Math.floor(Math.log2(segCount));
  const sub = new ByteWriter();
  for (const v of [
    4,
    16 + 8 * segCount,
    0,
    2 * segCount,
    2 * pow2,
    entrySelector,
    2 * segCount - 2 * pow2,
  ]) {
    sub.u16(v);
  }
  for (const [code] of mappings) sub.u16(code);
  sub.u16(0);
  for (const [code] of mappings) sub.u16(code);
  for (const [code, gid] of mappings) sub.u16((gid - code) & 0xffff);
  for (let i = 0; i < mappings.length; i++) sub.u16(0);

  const cmap = new ByteWriter();
  for (const n of [0, 1, 3, 1]) cmap.u16(n);
  cmap.u32(12);
  cmap.extend(sub.toUint8Array());

  // post table (32 bytes)
  const post = new ByteWriter();
  post.u32(0x00030000);
  post.u32(0);
  post.i16(-100);
  post.i16(50);
  for (let i = 0; i < 5; i++) post.u32(0);

  // name table
  const psName =
    family.replace(/[^A-Za-z0-9]/g, '').slice(0, 48) || 'NoteGenHandwriting';
  const nameRecords: Array<[number, string]> = [
    [1, family],
    [2, 'Regular'],
    [3, `NoteGen:${psName}:1`],
    [4, `${family} Regular`],
    [5, 'Version 1.000; NoteGen confirmed samples'],
    [6, `${psName}-Regular`],
  ];
  const nameTable = new ByteWriter();
  nameTable.u16(0);
  nameTable.u16(nameRecords.length);
  nameTable.u16(6 + 12 * nameRecords.length);
  const strStorage = new ByteWriter();
  for (const [nameId, text] of nameRecords) {
    const utf16 = new ByteWriter();
    for (let i = 0; i < text.length; i++) {
      utf16.u16(text.charCodeAt(i));
    }
    const bytes = utf16.toUint8Array();
    for (const n of [3, 1, 0x0409, nameId, bytes.length, strStorage.length]) {
      nameTable.u16(n);
    }
    strStorage.extend(bytes);
  }
  nameTable.extend(strStorage.toUint8Array());

  const tables: Array<[string, Uint8Array]> = [
    ['cmap', cmap.toUint8Array()],
    ['glyf', glyfWriter.toUint8Array()],
    ['head', head.toUint8Array()],
    ['hhea', hhea.toUint8Array()],
    ['hmtx', hmtxWriter.toUint8Array()],
    ['loca', locaWriter.toUint8Array()],
    ['maxp', maxp.toUint8Array()],
    ['name', nameTable.toUint8Array()],
    ['post', post.toUint8Array()],
  ];

  const numTables = tables.length;
  const maxPow = 1 << Math.floor(Math.log2(numTables));
  const out = new ByteWriter();
  out.u32(0x00010000);
  out.u16(numTables);
  out.u16(maxPow * 16);
  out.u16(Math.floor(Math.log2(numTables)));
  out.u16(numTables * 16 - maxPow * 16);

  let offset = 12 + numTables * 16;
  let headOffset = 0;
  for (const [tag, data] of tables) {
    for (let i = 0; i < 4; i++) out.u8(tag.charCodeAt(i));
    out.u32(computeTtfTableChecksum(data));
    out.u32(offset);
    out.u32(data.length);
    if (tag === 'head') headOffset = offset;
    offset += Math.ceil(data.length / 4) * 4;
  }

  for (const [, data] of tables) {
    out.extend(data);
    out.pad4();
  }

  const finalBytes = out.toUint8Array();
  const adjustment = (0xb1b0afba - computeTtfTableChecksum(finalBytes)) >>> 0;
  finalBytes[headOffset + 8] = (adjustment >>> 24) & 0xff;
  finalBytes[headOffset + 9] = (adjustment >>> 16) & 0xff;
  finalBytes[headOffset + 10] = (adjustment >>> 8) & 0xff;
  finalBytes[headOffset + 11] = adjustment & 0xff;

  return finalBytes;
}

export function generateWriterFontPack(
  profile: WriterProfile,
  options: FontPackOptions
): GeneratedFontPack {
  const family = options.family.trim();
  if (
    !family ||
    family.length > 100 ||
    !/[A-Za-z0-9]/.test(family) ||
    options.spacing < 0 ||
    options.spacing > 500 ||
    options.threshold < 32 ||
    options.threshold > 224
  ) {
    throw new Error('NG620: Invalid font family, spacing or threshold');
  }

  const uniqueChars = Array.from(
    new Set(
      profile.samples
        .filter((s) => s.glyph && s.label.length === 1)
        .map((s) => s.label)
    )
  ).sort();

  if (uniqueChars.length === 0) {
    throw new Error(
      'NG620: Label individual letters in Review -> Teach handwriting before exporting a font'
    );
  }

  const traced: TracedGlyph[] = [];
  for (const ch of uniqueChars) {
    const avgGrid = averageGlyphGrid(profile, ch);
    const { contours, advance, svgPath } = traceGridContours(
      avgGrid,
      options.threshold,
      ch,
      options.spacing
    );
    traced.push({
      char: ch,
      contours,
      advance,
      svgPath,
      averagedGrid: avgGrid,
    });
  }

  const allAlnum: string[] = [];
  for (let c = 65; c <= 90; c++) allAlnum.push(String.fromCharCode(c));
  for (let c = 97; c <= 122; c++) allAlnum.push(String.fromCharCode(c));
  for (let c = 48; c <= 57; c++) allAlnum.push(String.fromCharCode(c));

  const missingChars = allAlnum.filter((c) => !uniqueChars.includes(c));
  const ttfBytes = buildTrueTypeFont(traced, family);

  const readmeText = [
    `${family} — NoteGen handwriting font`,
    `${traced.length} confirmed glyphs; averaged up to 32 examples each.`,
    `Missing letters/numbers: ${missingChars.join('') || 'None (Full 62/62 Coverage)'}`,
    `Only explicitly labelled glyphs are mapped. Install Handwriting.ttf in system Fonts settings.`,
    `This is a personal print-style outline font; cursive joining and kerning are not inferred.`,
  ].join('\n');

  return {
    family,
    glyphCount: traced.length,
    coveredChars: uniqueChars,
    missingChars,
    ttfBytes,
    glyphs: traced,
    readmeText,
  };
}
