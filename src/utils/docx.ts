// Native OpenXML Word (.docx) structure & XML builder ported from rust/src/word.rs
// Generates real OOXML document.xml, styles.xml, tables with gridSpan/vMerge, and figure observation/interpretation blocks.

import { Block, PageDocument } from './engine';

export function escapeXml(text: string): string {
  return Array.from(text)
    .filter((ch) => {
      const code = ch.codePointAt(0) ?? 0;
      return (
        ch === '\t' ||
        ch === '\n' ||
        ch === '\r' ||
        (code >= 0x20 && code <= 0xd7ff) ||
        (code >= 0xe000 && code <= 0xfffd) ||
        code >= 0x10000
      );
    })
    .map((ch) => {
      switch (ch) {
        case '&':
          return '&amp;';
        case '<':
          return '&lt;';
        case '>':
          return '&gt;';
        case '"':
          return '&quot;';
        case "'":
          return '&apos;';
        default:
          return ch;
      }
    })
    .join('');
}

export function buildWordParagraph(text: string, style = 'Normal'): string {
  let out = `<w:p><w:pPr><w:pStyle w:val="${escapeXml(style)}"/></w:pPr>`;
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    if (i > 0) {
      out += '<w:r><w:br/></w:r>';
    }
    out += `<w:r><w:t xml:space="preserve">${escapeXml(lines[i])}</w:t></w:r>`;
  }
  out += '</w:p>';
  return out;
}

export function buildWordTable(b: Block): string {
  if (b.rows.length === 0 || b.rows[0].length === 0) return '';
  let o =
    '<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/><w:tblBorders>';
  for (const side of ['top', 'left', 'bottom', 'right', 'insideH', 'insideV']) {
    o += `<w:${side} w:val="single" w:sz="4" w:color="8793A1"/>`;
  }
  o += '</w:tblBorders></w:tblPr><w:tblGrid>';
  const cols = b.rows[0].length;
  for (let i = 0; i < cols; i++) {
    o += `<w:gridCol w:w="${Math.floor(9300 / cols)}"/>`;
  }
  o += '</w:tblGrid>';

  for (let r = 0; r < b.rows.length; r++) {
    const row = b.rows[r];
    o += '<w:tr>';
    if (r < b.header_rows) {
      o += '<w:trPr><w:tblHeader/></w:trPr>';
    }
    let c = 0;
    while (c < cols) {
      const merge = b.merges.find(
        (m) =>
          r >= m.row &&
          r < m.row + m.rowspan &&
          c >= m.col &&
          c < m.col + m.colspan
      );
      if (merge && c !== merge.col) {
        c += 1;
        continue;
      }
      o += '<w:tc><w:tcPr>';
      let advance = 1;
      let cellText = row[c] ?? '';
      if (merge) {
        advance = merge.colspan;
        if (merge.colspan > 1) {
          o += `<w:gridSpan w:val="${merge.colspan}"/>`;
        }
        if (merge.rowspan > 1) {
          o +=
            r === merge.row
              ? '<w:vMerge w:val="restart"/>'
              : '<w:vMerge/>';
        }
        if (r === merge.row) {
          const mergedParts: string[] = [];
          for (let rr = merge.row; rr < merge.row + merge.rowspan; rr++) {
            for (let cc = merge.col; cc < merge.col + merge.colspan; cc++) {
              const val = b.rows[rr]?.[cc] ?? '';
              if (val) mergedParts.push(val);
            }
          }
          cellText = mergedParts.join('\n');
        } else {
          cellText = '';
        }
      }
      o += '</w:tcPr>';
      o += buildWordParagraph(cellText, 'Normal');
      o += '</w:tc>';
      c += advance;
    }
    o += '</w:tr>';
  }
  o += '</w:tbl>';
  return o;
}

export function buildWordDocumentXml(
  pages: PageDocument[],
  options: {
    keepPageBreaks: boolean;
    preserveFigures: boolean;
    reviewThreshold: number;
  }
): string {
  let body = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>`;
  pages.forEach((page, index) => {
    if (index > 0 && options.keepPageBreaks) {
      body += '<w:p><w:r><w:br w:type="page"/></w:r></w:p>';
    }
    body += buildWordParagraph(
      `Source page ${page.page}${page.reviewed ? ' · reviewed' : ''}`,
      'Caption'
    );
    const needsReview =
      !page.reviewed &&
      (page.agreement < options.reviewThreshold ||
        page.blocks.some((b) => b.uncertain));
    if (needsReview) {
      body += buildWordParagraph(
        'REVIEW NEEDED — compare this content with the scan.',
        'Caption'
      );
    }
    for (const warning of page.warnings) {
      body += buildWordParagraph(warning, 'Caption');
    }
    for (const b of page.blocks) {
      if (b.kind === 'table') {
        body += buildWordTable(b);
      } else if (b.kind === 'figure') {
        if (options.preserveFigures) {
          body += buildWordParagraph(`Observed: ${b.description}`, 'Normal');
          if (b.interpretation.trim()) {
            body += buildWordParagraph(
              `Contextual interpretation: ${b.interpretation}`,
              'Normal'
            );
          }
          body += buildWordParagraph(
            'AI diagram interpretations require source verification.',
            'Caption'
          );
        }
      } else {
        const style = b.kind === 'heading' ? 'Heading1' : 'Normal';
        const text = `${b.text}${b.uncertain ? ' [uncertain]' : ''}`;
        body += buildWordParagraph(text, style);
      }
    }
  });
  body +=
    '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1080" w:right="1080" w:bottom="1080" w:left="1080"/></w:sectPr></w:body></w:document>';
  return body;
}

export interface ZipEntryInput {
  name: string;
  data: Uint8Array | string;
}

const CRC32_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let i = 0; i < 256; i++) {
    let c = i;
    for (let k = 0; k < 8; k++) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[i] = c >>> 0;
  }
  return table;
})();

export function computeCrc32(bytes: Uint8Array): number {
  let crc = 0xffffffff;
  for (let i = 0; i < bytes.length; i++) {
    crc = CRC32_TABLE[(crc ^ bytes[i]) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

export function buildZipBytes(entries: ZipEntryInput[]): Uint8Array {
  const encoder = new TextEncoder();
  const localParts: Uint8Array[] = [];
  const centralParts: Uint8Array[] = [];
  let offset = 0;

  for (const entry of entries) {
    const nameBytes = encoder.encode(entry.name);
    const contentBytes =
      typeof entry.data === 'string' ? encoder.encode(entry.data) : entry.data;
    const crc = computeCrc32(contentBytes);
    const size = contentBytes.byteLength;

    // Local file header (30 bytes + filename)
    const localHeader = new Uint8Array(30 + nameBytes.length);
    const lv = new DataView(localHeader.buffer);
    lv.setUint32(0, 0x04034b50, true); // Local file header signature
    lv.setUint16(4, 20, true); // Version needed to extract (2.0)
    lv.setUint16(6, 0x0800, true); // UTF-8 filename flag
    lv.setUint16(8, 0, true); // Compression method: 0 (STORE)
    lv.setUint16(10, 0x4800, true); // Mod time
    lv.setUint16(12, 0x5d5f, true); // Mod date (2026-10-31)
    lv.setUint32(14, crc, true);
    lv.setUint32(18, size, true);
    lv.setUint32(22, size, true);
    lv.setUint16(26, nameBytes.length, true);
    lv.setUint16(28, 0, true);
    localHeader.set(nameBytes, 30);

    localParts.push(localHeader, contentBytes);

    // Central directory header (46 bytes + filename)
    const centralHeader = new Uint8Array(46 + nameBytes.length);
    const cv = new DataView(centralHeader.buffer);
    cv.setUint32(0, 0x02014b50, true); // Central directory signature
    cv.setUint16(4, 20, true); // Version made by
    cv.setUint16(6, 20, true); // Version needed
    cv.setUint16(8, 0x0800, true); // UTF-8 flag
    cv.setUint16(10, 0, true); // Method: STORE
    cv.setUint16(12, 0x4800, true);
    cv.setUint16(14, 0x5d5f, true);
    cv.setUint32(16, crc, true);
    cv.setUint32(20, size, true);
    cv.setUint32(24, size, true);
    cv.setUint16(28, nameBytes.length, true);
    cv.setUint16(30, 0, true);
    cv.setUint16(32, 0, true);
    cv.setUint16(34, 0, true);
    cv.setUint16(36, 0, true);
    cv.setUint32(38, 0, true);
    cv.setUint32(42, offset, true);
    centralHeader.set(nameBytes, 46);

    centralParts.push(centralHeader);
    offset += localHeader.byteLength + size;
  }

  const centralSize = centralParts.reduce((s, p) => s + p.byteLength, 0);
  const eocd = new Uint8Array(22);
  const ev = new DataView(eocd.buffer);
  ev.setUint32(0, 0x06054b50, true); // End of central directory signature
  ev.setUint16(4, 0, true);
  ev.setUint16(6, 0, true);
  ev.setUint16(8, entries.length, true);
  ev.setUint16(10, entries.length, true);
  ev.setUint32(12, centralSize, true);
  ev.setUint32(16, offset, true);
  ev.setUint16(20, 0, true);

  const totalSize = offset + centralSize + eocd.byteLength;
  const out = new Uint8Array(totalSize);
  let pos = 0;
  for (const part of localParts) {
    out.set(part, pos);
    pos += part.byteLength;
  }
  for (const part of centralParts) {
    out.set(part, pos);
    pos += part.byteLength;
  }
  out.set(eocd, pos);
  return out;
}

export function buildDocxBinary(
  pages: PageDocument[],
  options: {
    keepPageBreaks: boolean;
    preserveFigures: boolean;
    reviewThreshold: number;
  }
): Uint8Array {
  const documentXml = buildWordDocumentXml(pages, options);
  const contentTypesXml = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
</Types>`;
  const relsXml = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>`;
  const docRelsXml = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rIdStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>`;
  const stylesXml = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri"/><w:sz w:val="22"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="Caption"><w:name w:val="caption"/><w:rPr><w:i/><w:color w:val="6E7785"/><w:sz w:val="18"/></w:rPr></w:style>
  <w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/></w:style>
</w:styles>`;

  return buildZipBytes([
    { name: '[Content_Types].xml', data: contentTypesXml },
    { name: '_rels/.rels', data: relsXml },
    { name: 'word/_rels/document.xml.rels', data: docRelsXml },
    { name: 'word/styles.xml', data: stylesXml },
    { name: 'word/document.xml', data: documentXml },
  ]);
}
