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
