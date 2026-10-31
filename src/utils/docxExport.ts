import { ProjectData, WriterProfile } from './types';

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

export function buildWordDocumentBlob(project: ProjectData): Blob {
  const font = escapeHtml(project.settings.font || 'Calibri');
  const fontSize = project.settings.font_size || 11;

  const pagesHtml = project.pages
    .map((page, pageIdx) => {
      const blocksHtml = page.content.blocks
        .map((block) => {
          if (block.kind === 'heading') {
            return `<h2 style="font-family:'${font}',sans-serif;color:#111827;margin:12pt 0 6pt 0;">${escapeHtml(block.text)}</h2>`;
          }
          if (block.kind === 'equation') {
            return `<p style="font-family:'Cambria Math','${font}',serif;font-style:italic;background:#f8fafc;padding:6pt 10pt;border-left:3pt solid #0d9488;">${escapeHtml(block.text)}</p>`;
          }
          if (block.kind === 'table') {
            const rowsHtml = block.rows
              .map((row, rIdx) => {
                const tag = rIdx < block.header_rows ? 'th' : 'td';
                const bg = rIdx < block.header_rows ? 'background:#f1f5f9;font-weight:bold;' : '';
                const cells = row
                  .map(
                    (cell) =>
                      `<${tag} style="border:1pt solid #334155;padding:6pt 10pt;${bg}">${escapeHtml(cell)}</${tag}>`
                  )
                  .join('');
                return `<tr>${cells}</tr>`;
              })
              .join('');
            return `<table style="border-collapse:collapse;width:100%;margin:10pt 0;font-family:'${font}',sans-serif;font-size:${fontSize}pt;">${rowsHtml}</table>`;
          }
          if (block.kind === 'figure') {
            if (!project.settings.preserve_figures) return '';
            return `<div style="border:1pt solid #94a3b8;padding:10pt;margin:10pt 0;background:#f8fafc;">
              <p style="margin:0 0 4pt 0;font-weight:bold;color:#0f172a;">[Embedded Source Diagram — Page ${page.page}]</p>
              <p style="margin:0 0 4pt 0;color:#334155;"><strong>Visible evidence:</strong> ${escapeHtml(block.description)}</p>
              <p style="margin:0;color:#475569;"><strong>Contextual interpretation:</strong> ${escapeHtml(block.interpretation)}</p>
            </div>`;
          }
          return `<p style="font-family:'${font}',sans-serif;font-size:${fontSize}pt;line-height:1.45;margin:6pt 0;">${escapeHtml(block.text)}</p>`;
        })
        .join('\n');

      const pageBreak =
        project.settings.keep_page_breaks && pageIdx < project.pages.length - 1
          ? '<br clear="all" style="page-break-before:always" />'
          : '<hr style="margin:18pt 0;border:none;border-top:1pt solid #cbd5e1;" />';

      return `<section>
        <p style="font-size:9pt;color:#64748b;margin-bottom:8pt;">NoteGen Local Studio · Page ${page.page} · Agreement ${(page.agreement * 100).toFixed(1)}%</p>
        ${blocksHtml}
      </section>${pageBreak}`;
    })
    .join('\n');

  const html = `<html xmlns:o="urn:schemas-microsoft-com:office:office"
        xmlns:w="urn:schemas-microsoft-com:office:word"
        xmlns="http://www.w3.org/TR/REC-html40">
    <head>
      <meta charset="utf-8" />
      <title>${escapeHtml(project.name)}</title>
    </head>
    <body style="font-family:'${font}',sans-serif;font-size:${fontSize}pt;color:#0f172a;">
      ${pagesHtml}
    </body>
  </html>`;

  return new Blob([html], {
    type: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  });
}

export function triggerBlobDownload(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export function exportWriterPackBlob(profile: WriterProfile, fontFamily?: string): Blob {
  const payload = {
    exportedAt: new Date().toISOString(),
    generator: 'NoteGen Local Studio 0.3.0',
    fontFamily: fontFamily || `${profile.name} handwriting`,
    profile,
  };
  return new Blob([JSON.stringify(payload, null, 2)], { type: 'application/json' });
}
