import {
  Answer,
  Fault,
  Page,
  ProjectData,
  Question,
  Settings,
  WriterProfile,
  blockPlain,
  defaultSettings,
  isBoxValid,
} from './types';

export function createScanSvgDataUrl(pageNumber: number): string {
  if (pageNumber === 1) {
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="800" height="1040" viewBox="0 0 800 1040">
      <rect width="800" height="1040" fill="#ffffff"/>
      <text x="38" y="84" font-family="Verdana, sans-serif" font-size="22" fill="#182026">NoteGen scanned page</text>
      <text x="38" y="135" font-family="Verdana, sans-serif" font-size="21" fill="#182026">Revenue is 1250 in October.</text>
      <g stroke="#111111" stroke-width="2.5" fill="none">
        <rect x="38" y="278" width="690" height="168"/>
        <line x1="383" y1="278" x2="383" y2="446"/>
        <line x1="38" y1="362" x2="728" y2="362"/>
      </g>
      <text x="62" y="322" font-family="Verdana, sans-serif" font-size="21" fill="#182026">Item</text>
      <text x="408" y="322" font-family="Verdana, sans-serif" font-size="21" fill="#182026">Amount</text>
      <text x="62" y="405" font-family="Verdana, sans-serif" font-size="21" fill="#182026">Sales</text>
      <text x="408" y="405" font-family="Verdana, sans-serif" font-size="21" fill="#182026">1250</text>
      <rect x="70" y="590" width="620" height="340" fill="#fcfdfe" stroke="#d9e2ec" stroke-width="1.5"/>
      <line x1="82" y1="600" x2="82" y2="910" stroke="#111111" stroke-width="3"/>
      <line x1="82" y1="910" x2="670" y2="910" stroke="#111111" stroke-width="3"/>
      <rect x="206" y="630" width="102" height="280" fill="#2b5f8c"/>
      <rect x="394" y="720" width="102" height="190" fill="#4885b4"/>
      <text x="228" y="936" font-family="Verdana, sans-serif" font-size="16" fill="#334e68">Oct Q4</text>
      <text x="415" y="936" font-family="Verdana, sans-serif" font-size="16" fill="#334e68">Sep Q3</text>
    </svg>`;
    return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
  }

  const svg2 = `<svg xmlns="http://www.w3.org/2000/svg" width="800" height="1040" viewBox="0 0 800 1040">
    <rect width="800" height="1040" fill="#fdfbf7"/>
    <text x="44" y="82" font-family="Georgia, serif" font-style="italic" font-size="24" fill="#1e293b">Fluid Mechanics — Siphon &amp; Tank Balance</text>
    <text x="44" y="134" font-family="Comic Sans MS, cursive, sans-serif" font-size="20" fill="#27364b">Flow rate Q = A · v = 0.042 m³/s at h = 1.85 m</text>
    <g stroke="#1e293b" stroke-width="2" fill="none">
      <rect x="44" y="210" width="680" height="156"/>
      <line x1="270" y1="210" x2="270" y2="366"/>
      <line x1="500" y1="210" x2="500" y2="366"/>
      <line x1="44" y1="262" x2="724" y2="262"/>
      <line x1="44" y1="314" x2="724" y2="314"/>
    </g>
    <text x="64" y="244" font-family="sans-serif" font-size="18" font-weight="bold" fill="#1e293b">Point</text>
    <text x="290" y="244" font-family="sans-serif" font-size="18" font-weight="bold" fill="#1e293b">Pressure (kPa)</text>
    <text x="520" y="244" font-family="sans-serif" font-size="18" font-weight="bold" fill="#1e293b">Velocity (m/s)</text>
    <text x="64" y="296" font-family="Comic Sans MS, cursive, sans-serif" font-size="18" fill="#1e293b">Tank A Surface</text>
    <text x="290" y="296" font-family="Comic Sans MS, cursive, sans-serif" font-size="18" fill="#1e293b">101.3</text>
    <text x="520" y="296" font-family="Comic Sans MS, cursive, sans-serif" font-size="18" fill="#1e293b">0.00</text>
    <text x="64" y="348" font-family="Comic Sans MS, cursive, sans-serif" font-size="18" fill="#1e293b">Crest Summit B</text>
    <text x="290" y="348" font-family="Comic Sans MS, cursive, sans-serif" font-size="18" fill="#1e293b">78.4</text>
    <text x="520" y="348" font-family="Comic Sans MS, cursive, sans-serif" font-size="18" fill="#1e293b">6.02</text>
    <rect x="60" y="440" width="640" height="320" fill="#ffffff" stroke="#94a3b8" stroke-width="1.5"/>
    <rect x="110" y="540" width="160" height="160" fill="#bae6fd" stroke="#0284c7" stroke-width="2.5"/>
    <rect x="470" y="610" width="160" height="90" fill="#bae6fd" stroke="#0284c7" stroke-width="2.5"/>
    <path d="M 190 580 C 190 460, 550 460, 550 640" fill="none" stroke="#0f172a" stroke-width="6"/>
    <text x="155" y="630" font-family="sans-serif" font-size="18" fill="#0369a1">Tank A</text>
    <text x="515" y="665" font-family="sans-serif" font-size="18" fill="#0369a1">Tank B</text>
    <text x="340" y="480" font-family="Comic Sans MS, cursive, sans-serif" font-size="17" fill="#0f172a">Summit h_c = 1.85m</text>
  </svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg2)}`;
}

export function createSamplePages(filename: string): Page[] {
  const isFluid = filename.toLowerCase().includes('fluid') || filename.toLowerCase().includes('lecture');
  const page1: Page = {
    edited: false,
    page: 1,
    image: createScanSvgDataUrl(1),
    agreement: 0.84,
    reviewed: false,
    seconds: 3.4,
    source_warnings: [],
    content: {
      summary: 'Scanned accounting ledger and revenue summary bar chart for October.',
      warnings: [
        '4 source regions could not be located. Uncertain figures retain the full source page; verify their boundaries in Review.',
        "A reader omitted a structured region; another reader's table/figure was retained for verification.",
        'Independent readers disagree. Selected an existing reading; alternatives are preserved for review.',
        'Difficult-region crop readings are preserved in evidence. Review conflicting regions.',
        'Independent readers disagree. Selected an existing reading; alternatives are preserved for review.',
      ],
      blocks: [
        {
          kind: 'text',
          text: 'NoteGen scanned page',
          box: { left: 0.045, top: 0.06, right: 0.242, bottom: 0.088 },
          confidence: 0.96,
          rows: [],
          merges: [],
          header_rows: 0,
          description: '',
          interpretation: '',
          uncertain: true,
          alternatives: ['lighton3-4b: NoteGen scanned page', 'qwen35-9b: NoteGen scanned page'],
        },
        {
          kind: 'text',
          text: 'Revenue is 1250 in October.',
          box: { left: 0.045, top: 0.105, right: 0.28, bottom: 0.138 },
          confidence: 0.89,
          rows: [],
          merges: [],
          header_rows: 0,
          description: '',
          interpretation: '',
          uncertain: true,
          alternatives: [
            'lighton3-4b: Revenue is 1250 in October.',
            'glm-ocr: Revenue is 1250 in October.',
            'qwen35-9b: Revenue is 1,250 in October.',
          ],
        },
        {
          kind: 'table',
          text: '',
          box: { left: 0.045, top: 0.265, right: 0.912, bottom: 0.432 },
          confidence: 0.94,
          rows: [
            ['Item', 'Amount'],
            ['Sales', '1250'],
          ],
          merges: [],
          header_rows: 1,
          description: '',
          interpretation: '',
          uncertain: false,
          alternatives: ['lighton3-4b: Item\tAmount\nSales\t1250'],
        },
        {
          kind: 'figure',
          text: '',
          box: { left: 0.085, top: 0.565, right: 0.865, bottom: 0.915 },
          confidence: 0.88,
          rows: [],
          merges: [],
          header_rows: 0,
          description:
            'Two-column vertical bar chart on a bordered cartesian axis showing primary tall blue column (Oct Q4) and secondary medium blue column (Sep Q3).',
          interpretation:
            'Illustrates October revenue growth (1250 units) relative to prior period baseline in the sales table.',
          uncertain: true,
          alternatives: [],
        },
        {
          kind: 'figure',
          text: '',
          box: { left: 0.195, top: 0.595, right: 0.64, bottom: 0.89 },
          confidence: 0.82,
          rows: [],
          merges: [],
          header_rows: 0,
          description: 'Primary bar column region aligned with Sales = 1250.',
          interpretation: 'Peak monthly sales bar highlighted for financial review.',
          uncertain: true,
          alternatives: [],
        },
      ],
    },
    candidates: [
      {
        model_id: 'lighton3-4b',
        family: 'lighton3',
        view: 'full',
        reliability: 0.96,
        content: {
          summary: 'LightOnOCR-3 4B full page transcript',
          warnings: [],
          blocks: [
            {
              kind: 'text',
              text: 'NoteGen scanned page\nRevenue is 1250 in October.',
              box: { left: 0.045, top: 0.06, right: 0.28, bottom: 0.14 },
              rows: [],
              merges: [],
              header_rows: 0,
              description: '',
              interpretation: '',
              uncertain: false,
              alternatives: [],
            },
            {
              kind: 'table',
              text: '',
              box: { left: 0.045, top: 0.265, right: 0.912, bottom: 0.432 },
              rows: [
                ['Item', 'Amount'],
                ['Sales', '1250'],
              ],
              merges: [],
              header_rows: 1,
              description: '',
              interpretation: '',
              uncertain: false,
              alternatives: [],
            },
          ],
        },
      },
      {
        model_id: 'glm-ocr',
        family: 'glm',
        view: 'full',
        reliability: 0.91,
        content: {
          summary: 'GLM-OCR 0.9B full page transcript',
          warnings: [],
          blocks: [
            {
              kind: 'text',
              text: 'NoteGen scanned page\nRevenue is 1250 in October.',
              rows: [],
              merges: [],
              header_rows: 0,
              description: '',
              interpretation: '',
              uncertain: false,
              alternatives: [],
            },
          ],
        },
      },
      {
        model_id: 'qwen35-9b',
        family: 'qwen35',
        view: 'full',
        reliability: 0.94,
        content: {
          summary: 'Qwen3.5 9B structured vision reading',
          warnings: [],
          blocks: [
            {
              kind: 'text',
              text: 'NoteGen scanned page\nRevenue is 1250 in October.',
              rows: [],
              merges: [],
              header_rows: 0,
              description: '',
              interpretation: '',
              uncertain: false,
              alternatives: [],
            },
            {
              kind: 'figure',
              text: '',
              box: { left: 0.085, top: 0.565, right: 0.865, bottom: 0.915 },
              rows: [],
              merges: [],
              header_rows: 0,
              description: 'Vertical bar chart showing October revenue column.',
              interpretation: 'Visual comparison of Sales amount 1250.',
              uncertain: true,
              alternatives: [],
            },
          ],
        },
      },
      {
        model_id: 'tesseract',
        family: 'tesseract',
        view: 'full',
        reliability: 0.82,
        content: {
          summary: 'Tesseract printed baseline',
          warnings: [],
          blocks: [
            {
              kind: 'text',
              text: 'NoteGen scanned page\nRevenue is 1250 in October.\nItem Amount\nSales 1250',
              rows: [],
              merges: [],
              header_rows: 0,
              description: '',
              interpretation: '',
              uncertain: false,
              alternatives: [],
            },
          ],
        },
      },
    ],
  };

  const page2: Page = {
    edited: false,
    page: 2,
    image: createScanSvgDataUrl(2),
    agreement: 0.91,
    reviewed: false,
    seconds: 4.1,
    source_warnings: [],
    content: {
      summary: 'Handwritten fluid mechanics notes with siphon pressure table and tank diagram.',
      warnings: [
        'Handwritten velocity symbol v closely resembles r; confirmed against writer profile.',
      ],
      blocks: [
        {
          kind: 'heading',
          text: 'Fluid Mechanics — Siphon & Tank Balance',
          box: { left: 0.05, top: 0.055, right: 0.68, bottom: 0.095 },
          confidence: 0.95,
          rows: [],
          merges: [],
          header_rows: 0,
          description: '',
          interpretation: '',
          uncertain: false,
          alternatives: [],
        },
        {
          kind: 'equation',
          text: 'Flow rate Q = A · v = 0.042 m³/s at h = 1.85 m',
          box: { left: 0.05, top: 0.11, right: 0.65, bottom: 0.15 },
          confidence: 0.87,
          rows: [],
          merges: [],
          header_rows: 0,
          description: '',
          interpretation: '',
          uncertain: true,
          alternatives: [
            'lighton3-4b: Flow rate Q = A · v = 0.042 m³/s at h = 1.85 m',
            'qwen35-9b: Flow rate Q = A · r = 0.042 m³/s at h = 1.85 m',
          ],
        },
        {
          kind: 'table',
          text: '',
          box: { left: 0.055, top: 0.2, right: 0.905, bottom: 0.355 },
          confidence: 0.93,
          rows: [
            ['Point', 'Pressure (kPa)', 'Velocity (m/s)'],
            ['Tank A Surface', '101.3', '0.00'],
            ['Crest Summit B', '78.4', '6.02'],
          ],
          merges: [],
          header_rows: 1,
          description: '',
          interpretation: '',
          uncertain: false,
          alternatives: [],
        },
        {
          kind: 'figure',
          text: '',
          box: { left: 0.075, top: 0.42, right: 0.875, bottom: 0.735 },
          confidence: 0.9,
          rows: [],
          merges: [],
          header_rows: 0,
          description:
            'Two open reservoirs (Tank A elevated on left, Tank B lower on right) connected by an inverted U-tube siphon reaching crest h_c = 1.85m.',
          interpretation:
            'Demonstrates sub-atmospheric pressure at Crest Summit B (78.4 kPa) driving steady gravitational siphon flow.',
          uncertain: true,
          alternatives: [],
        },
      ],
    },
    candidates: [
      {
        model_id: 'lighton3-4b',
        family: 'lighton3',
        view: 'full',
        reliability: 0.95,
        content: {
          summary: 'LightOnOCR-3 4B reading',
          warnings: [],
          blocks: [
            {
              kind: 'text',
              text: 'Fluid Mechanics — Siphon & Tank Balance\nFlow rate Q = A · v = 0.042 m³/s at h = 1.85 m',
              rows: [],
              merges: [],
              header_rows: 0,
              description: '',
              interpretation: '',
              uncertain: false,
              alternatives: [],
            },
          ],
        },
      },
      {
        model_id: 'qwen35-9b',
        family: 'qwen35',
        view: 'full',
        reliability: 0.93,
        content: {
          summary: 'Qwen3.5 9B reading',
          warnings: [],
          blocks: [
            {
              kind: 'text',
              text: 'Fluid Mechanics — Siphon & Tank Balance\nFlow rate Q = A · v = 0.042 m³/s at h = 1.85 m',
              rows: [],
              merges: [],
              header_rows: 0,
              description: '',
              interpretation: '',
              uncertain: false,
              alternatives: [],
            },
          ],
        },
      },
    ],
  };

  return isFluid ? [page2, page1] : [page1, page2];
}

export function createInitialProjects(): ProjectData[] {
  const settings = defaultSettings();
  return [
    {
      id: 'proj-october-revenue',
      name: 'october-revenue-scan.notegen',
      input: 'october-revenue-scan.pdf',
      output: 'october-revenue-scan.docx',
      settings,
      pages: createSamplePages('october-revenue-scan.pdf'),
      notes: [],
      answers: [],
    },
  ];
}

export function createInitialWriters(): WriterProfile[] {
  const glyphs = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz0123456789'.split('');
  return [
    {
      schema: 1,
      id: 'writer-induwara-lab',
      name: 'Induwara Engineering Lab',
      notes: 'Cursive lowercase v has a rounded tail; Q always has an underscore cross-bar.',
      revision: 14,
      samples: [
        ...glyphs.slice(0, 28).map((ch, idx) => ({
          id: `sample-glyph-${idx}`,
          label: ch,
          glyph: true,
          image: `sample-glyph-${idx}.png`,
          created: '2026-10-30T14:22:00Z',
          source: 'october-revenue-scan.notegen page 2',
        })),
        {
          id: 'sample-word-flow',
          label: 'Flow rate Q',
          glyph: false,
          image: 'sample-word-flow.png',
          created: '2026-10-30T15:10:00Z',
          source: 'october-revenue-scan.notegen page 2',
        },
        {
          id: 'sample-word-siphon',
          label: 'Crest Summit B',
          glyph: false,
          image: 'sample-word-siphon.png',
          created: '2026-10-30T15:12:00Z',
          source: 'october-revenue-scan.notegen page 2',
        },
      ],
    },
  ];
}

export function writerCoverage(profile: WriterProfile): string {
  const chars =
    'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789'.split('');
  return chars
    .filter((ch) => profile.samples.some((s) => s.glyph && s.label === ch))
    .join('');
}

export function generateQuestions(
  project: ProjectData,
  limit: number,
  threshold: number
): Question[] {
  const out: Question[] = [];
  for (const page of project.pages) {
    if (page.reviewed) continue;
    for (let index = 0; index < page.content.blocks.length; index++) {
      if (out.length >= limit) return out;
      const block = page.content.blocks[index];
      const prefix = `p${page.page}-b${index}-`;
      if (
        project.answers.some(
          (a) => !a.skip && a.replacement.trim().length > 0 && a.id.startsWith(prefix)
        )
      ) {
        continue;
      }
      const needsClarification =
        block.uncertain ||
        (!page.edited && block.confidence !== undefined && block.confidence < threshold) ||
        blockPlain(block).includes('[illegible]') ||
        (block.alternatives.length > 0 && page.agreement < 0.92);
      if (!needsClarification) continue;

      let field = 'text';
      let reading = block.text;
      let prompt =
        'What does this handwriting or unclear text say? Enter the exact words, or add context and leave the correction empty.';

      if (block.kind === 'figure') {
        field = 'meaning';
        reading = block.interpretation;
        prompt =
          'What does this picture or diagram represent? Explain its topic, labels or relationships you know.';
      } else if (block.kind === 'table') {
        let target: [number, number, string] | null = null;
        for (let r = 0; r < block.rows.length; r++) {
          for (let c = 0; c < block.rows[r].length; c++) {
            const cell = block.rows[r][c];
            if (cell.includes('[illegible]') || (!target && cell.length > 0)) {
              target = [r, c, cell];
              if (cell.includes('[illegible]')) break;
            }
          }
        }
        if (!target) continue;
        field = `cell:${target[0]}:${target[1]}`;
        reading = target[2];
        prompt = `Confirm table row ${target[0] + 1}, column ${target[1] + 1}. Enter the exact cell, including digits, units and signs.`;
      } else if (/\d/.test(reading)) {
        prompt =
          'Can you confirm these words and numbers? Enter the exact text, preserving all digits and signs.';
      }

      out.push({
        id: `p${page.page}-b${index}-${block.kind}`,
        page: page.page,
        block: index,
        field,
        prompt,
        reading,
        alternatives: block.alternatives.slice(0, 8),
        region: block.box,
        image: page.image,
        block_digest: `${block.kind}:${reading.length}`,
      });
    }
  }
  return out;
}

export function validateClarificationAnswers(questions: Question[], answers: Answer[]): void {
  if (answers.length > 32) {
    throw new Fault('NG630', 'Too many answers');
  }
  const ids = new Set<string>();
  for (const a of answers) {
    const q = questions.find((item) => item.id === a.id);
    if (!q) {
      throw new Fault('NG630', 'Unknown clarification question');
    }
    if (ids.has(a.id)) {
      throw new Fault('NG630', 'Duplicate clarification answer');
    }
    ids.add(a.id);
    if (a.skip) continue;
    if (a.replacement.length > 120000 || a.context.length > 4000) {
      throw new Fault('NG630', 'Clarification is too long');
    }
    if (a.learn && a.replacement.trim().length > 0) {
      const label = a.replacement.trim();
      const validLearn =
        q.field === 'text' &&
        q.region !== undefined &&
        isBoxValid(q.region) &&
        label.length <= 160 &&
        (!a.glyph || (label.length === 1 && /^[\x21-\x7E]$/.test(label)));
      if (!validLearn) {
        throw new Fault(
          'NG630',
          'To remember a crop, label a short word/phrase or one isolated English glyph. Use Review to draw a tighter region.'
        );
      }
    }
  }
}
