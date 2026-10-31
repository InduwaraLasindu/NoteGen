// Pure, deterministic NoteGen 0.3.1 recognition, consensus, clarification, writer memory, and model management logic.
// Decoupled from React UI so every subsystem can be batch-tested via node:test (`npx --yes tsx --test src/utils/engine.test.ts`).

export type Preset = 'fast' | 'balanced' | 'accuracy' | 'maximum';
export type Backend = 'auto' | 'cuda' | 'vulkan' | 'cpu';

export interface BoxRect {
  left: number;   // 0..1 normalized
  top: number;    // 0..1 normalized
  right: number;  // 0..1 normalized
  bottom: number; // 0..1 normalized
}

export interface TableMerge {
  row: number;
  col: number;
  rowspan: number;
  colspan: number;
}

export interface Block {
  id: string;
  kind: 'heading' | 'text' | 'table' | 'figure';
  text: string;
  rows: string[][];
  header_rows: number;
  merges: TableMerge[];
  description: string;
  interpretation: string;
  bbox: BoxRect | null;
  confidence: number;
  uncertain: boolean;
  alternatives: string[];
}

export interface CandidateReading {
  model_id: string;
  family: string;
  view: 'full' | 'crop';
  reliability: number;
  plain_text: string;
  blocks: Block[];
}

export interface PageDocument {
  page: number;
  title: string;
  image_id: string;
  blocks: Block[];
  candidates: CandidateReading[];
  agreement: number;
  warnings: string[];
  edited: boolean;
  reviewed: boolean;
}

export interface ClarificationQuestion {
  id: string;
  page: number;
  blockIndex: number;
  field: string; // 'text' | 'meaning' | 'cell:r:c'
  prompt: string;
  reading: string;
  alternatives: string[];
  region: BoxRect | null;
  blockDigest: string;
}

export interface ClarificationAnswer {
  id: string;
  replacement: string;
  context: string;
  skip: boolean;
  learn: boolean;
  glyph: boolean;
}

export interface WriterSample {
  id: string;
  label: string;
  glyph: boolean;
  created: string;
  source: string;
  // 16x20 normalized ink grid (0 = white paper, 255 = solid ink) for deterministic glyph averaging & contour tracing
  inkGrid: number[];
  strokeTrait: string;
}

export interface WriterProfile {
  schema: number;
  id: string;
  name: string;
  notes: string;
  revision: number;
  samples: WriterSample[];
}

export interface ReaderModel {
  id: string;
  name: string;
  family: string;
  role: 'ocr' | 'vision';
  prompt: 'structured' | 'lighton' | 'glm';
  resident_mib: number;
  weights_mib: number;
  rank: number;
  source: string;
  released: string;
  revision: string;
  previous_revision?: string;
  sha256: string;
  endpoint?: string;
  alias: string;
  hub?: {
    repository: string;
    weights: string;
    projector: string;
    available_revision?: string;
    available_sha256?: string;
    available_bytes?: number;
  };
  enabled: boolean;
  custom?: boolean;
}

export interface WebSource {
  title: string;
  url: string;
  excerpt: string;
  retrieved: string;
}

export interface AppearanceSettings {
  dark: boolean;
  accent: [number, number, number];
  scale: number;
  motion: number;
  scan_animation: boolean;
  intro_animation: boolean;
  hover_effects: boolean;
  transition_ms: number;
  ambient_density: number;
  idle_fps: number;
  reduced_motion: boolean;
  particles: boolean;
  glow: boolean;
  rounding: number;
  fps: number;
  compact: boolean;
}

// ---------------------------------------------------------------------------
// 1. String & Number Normalization + Levenshtein Similarity (rust/src/consensus.rs)
// ---------------------------------------------------------------------------

export function normalizeText(s: string): string {
  return s
    .normalize('NFKC')
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .join(' ');
}

export function levenshteinDistance(a: string, b: string): number {
  const s = normalizeText(a);
  const t = normalizeText(b);
  if (s === t) return 0;
  if (s.length === 0) return t.length;
  if (t.length === 0) return s.length;

  let prev = Array.from({ length: t.length + 1 }, (_, i) => i);
  let curr = new Array<number>(t.length + 1);

  for (let i = 0; i < s.length; i++) {
    curr[0] = i + 1;
    for (let j = 0; j < t.length; j++) {
      const cost = s[i] === t[j] ? 0 : 1;
      curr[j + 1] = Math.min(curr[j] + 1, prev[j + 1] + 1, prev[j] + cost);
    }
    const tmp = prev;
    prev = curr;
    curr = tmp;
  }
  return prev[t.length];
}

export function textSimilarity(a: string, b: string): number {
  const s = normalizeText(a);
  const t = normalizeText(b);
  const maxLen = Math.max(s.length, t.length);
  if (maxLen === 0) return 1.0;
  return Math.max(0, 1 - levenshteinDistance(s, t) / maxLen);
}

export function extractNumbers(text: string): string[] {
  const tokens = text
    .split(/\s+/)
    .filter((t) => /\d/.test(t))
    .map((t) => t.replace(/^[^0-9+-]+|[^0-9]+$/g, ''))
    .filter((t) => /\d/.test(t));
  return Array.from(new Set(tokens)).sort();
}

export function boxOverlap(a: BoxRect, b: BoxRect): number {
  const interW = Math.max(0, Math.min(a.right, b.right) - Math.max(a.left, b.left));
  const interH = Math.max(0, Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top));
  const inter = interW * interH;
  const areaA = Math.max(0, a.right - a.left) * Math.max(0, a.bottom - a.top);
  const areaB = Math.max(0, b.right - b.left) * Math.max(0, b.bottom - b.top);
  const union = areaA + areaB - inter;
  return union > 0 ? inter / union : 0;
}

// ---------------------------------------------------------------------------
// 2. Independent Family Medoid Consensus (rust/src/consensus.rs)
// ---------------------------------------------------------------------------

export interface ConsensusResult {
  chosenModelId: string;
  chosenFamily: string;
  independentFamilies: number;
  agreement: number;
  numericConflict: boolean;
  warnings: string[];
  blocks: Block[];
}

export function selectConsensus(
  candidates: CandidateReading[],
  minimumFamilies = 2,
  threshold = 0.92
): ConsensusResult {
  const fullCandidates = candidates.filter((c) => c.view === 'full');
  if (fullCandidates.length === 0) {
    return {
      chosenModelId: 'none',
      chosenFamily: 'none',
      independentFamilies: 0,
      agreement: 0,
      numericConflict: false,
      warnings: ['No recognizer returned usable content.'],
      blocks: [],
    };
  }

  // Deduplicate by independent family: highest reliability per family wins that family's single vote
  const familyMap = new Map<string, CandidateReading>();
  for (const c of fullCandidates) {
    const existing = familyMap.get(c.family);
    if (!existing || c.reliability > existing.reliability) {
      familyMap.set(c.family, c);
    }
  }

  const voters = Array.from(familyMap.values()).sort((a, b) =>
    a.model_id.localeCompare(b.model_id)
  );

  // Weighted medoid selection
  let bestIdx = 0;
  let bestScore = -1;
  const weightSum = voters.reduce(
    (acc, v) => acc + Math.min(1, Math.max(0.1, v.reliability)),
    0
  );

  for (let i = 0; i < voters.length; i++) {
    const candidate = voters[i];
    const score =
      voters.reduce((acc, other) => {
        const w = Math.min(1, Math.max(0.1, other.reliability));
        return acc + textSimilarity(candidate.plain_text, other.plain_text) * w;
      }, 0) / Math.max(0.1, weightSum);

    if (
      score > bestScore ||
      (Math.abs(score - bestScore) < 1e-9 &&
        candidate.reliability > voters[bestIdx].reliability)
    ) {
      bestIdx = i;
      bestScore = score;
    }
  }

  const chosen = voters[bestIdx];
  const warnings: string[] = [];

  const agreement =
    voters.length < minimumFamilies
      ? 0
      : voters
          .filter((v) => v.model_id !== chosen.model_id)
          .reduce(
            (acc, v) => acc + textSimilarity(chosen.plain_text, v.plain_text),
            0
          ) / Math.max(1, voters.length - 1);

  if (voters.length < minimumFamilies) {
    warnings.push(
      `Only ${voters.length} independent recognizer families; ${minimumFamilies} required for consensus.`
    );
  }

  if (agreement < threshold && voters.length >= minimumFamilies) {
    warnings.push(
      'Independent readers disagree. Selected an existing reading; alternatives are preserved for review.'
    );
  }

  // Numeric conflict detection across independent families
  const expectedNums = extractNumbers(chosen.plain_text).join('|');
  const numericConflict = voters.some(
    (v) => extractNumbers(v.plain_text).join('|') !== expectedNums
  );
  if (numericConflict) {
    warnings.push(
      'Numeric disagreement: check every conflicting amount, date, unit or equation against the scan.'
    );
  }

  const clonedBlocks: Block[] = chosen.blocks.map((b) => ({
    ...b,
    rows: b.rows.map((r) => [...r]),
    merges: b.merges.map((m) => ({ ...m })),
    alternatives: [...b.alternatives],
    uncertain:
      b.uncertain ||
      agreement < threshold ||
      numericConflict ||
      b.text.toLowerCase().includes('[illegible]'),
  }));

  return {
    chosenModelId: chosen.model_id,
    chosenFamily: chosen.family,
    independentFamilies: voters.length,
    agreement,
    numericConflict,
    warnings,
    blocks: clonedBlocks,
  };
}

// ---------------------------------------------------------------------------
// 3. Deterministic Block Fingerprint & Clarification Engine (rust/src/assistance.rs)
// ---------------------------------------------------------------------------

export function blockPlain(b: Block): string {
  if (b.kind === 'table') {
    return b.rows.map((r) => r.join(' | ')).join('\n');
  }
  if (b.kind === 'figure') {
    return `${b.description} ${b.interpretation}`.trim();
  }
  return b.text;
}

export function computeDigest(input: string): string {
  // Deterministic 64-bit FNV-1a hex digest for fast browser + Node test parity
  let h1 = 0x811c9dc5;
  let h2 = 0xcbf29ce4;
  for (let i = 0; i < input.length; i++) {
    const ch = input.charCodeAt(i);
    h1 ^= ch;
    h1 = Math.imul(h1, 0x01000193);
    h2 ^= ch ^ (i & 0xff);
    h2 = Math.imul(h2, 0x01000193);
  }
  return (
    (h1 >>> 0).toString(16).padStart(8, '0') +
    (h2 >>> 0).toString(16).padStart(8, '0')
  );
}

export function blockDigest(b: Block): string {
  return computeDigest(
    JSON.stringify({
      kind: b.kind,
      text: b.text,
      rows: b.rows,
      description: b.description,
      interpretation: b.interpretation,
    })
  );
}

export function buildClarificationQuestions(
  pages: PageDocument[],
  existingAnswers: ClarificationAnswer[],
  limit = 8,
  reviewThreshold = 0.9
): ClarificationQuestion[] {
  const out: ClarificationQuestion[] = [];
  for (const page of pages) {
    if (page.reviewed) continue;
    for (let index = 0; index < page.blocks.length; index++) {
      if (out.length >= limit) return out;
      const block = page.blocks[index];
      const digest = blockDigest(block);
      const qid = `p${page.page}-b${index}-${digest.slice(0, 10)}`;

      if (
        existingAnswers.some(
          (a) => !a.skip && a.replacement.trim().length > 0 && a.id === qid
        )
      ) {
        continue;
      }

      const plain = blockPlain(block);
      const distinctAlternativeTexts = new Set(
        block.alternatives.map((alt) =>
          normalizeText(alt.replace(/^[a-z0-9_-]+:\s*/i, ''))
        )
      );
      const needsClarification =
        block.uncertain ||
        (!page.edited && block.confidence < reviewThreshold) ||
        plain.includes('[illegible]') ||
        (distinctAlternativeTexts.size > 1 &&
          page.agreement < 0.92 &&
          block.confidence < reviewThreshold);

      if (!needsClarification) continue;

      let field = 'text';
      let reading = block.text;
      let prompt =
        'What does this handwriting or unclear text say? Enter the exact words, or add context and leave the correction empty.';

      if (block.kind === 'figure') {
        field = 'meaning';
        reading = block.interpretation || block.description;
        prompt =
          'What does this picture or diagram represent? Explain its topic, labels or relationships you know.';
      } else if (block.kind === 'table') {
        let foundCell: { r: number; c: number; text: string } | null = null;
        for (let r = 0; r < block.rows.length; r++) {
          for (let c = 0; c < block.rows[r].length; c++) {
            const cell = block.rows[r][c];
            if (cell.includes('[illegible]') || /\d/.test(cell)) {
              foundCell = { r, c, text: cell };
              if (cell.includes('[illegible]')) break;
            }
          }
          if (foundCell?.text.includes('[illegible]')) break;
        }
        if (!foundCell) continue;
        field = `cell:${foundCell.r}:${foundCell.c}`;
        reading = foundCell.text;
        prompt = `Confirm table row ${foundCell.r + 1}, column ${foundCell.c + 1}. Enter the exact cell, including digits, units and signs.`;
      } else if (/\d/.test(reading)) {
        prompt =
          'Can you confirm these words and numbers? Enter the exact text, preserving all digits and signs.';
      }

      out.push({
        id: qid,
        page: page.page,
        blockIndex: index,
        field,
        prompt,
        reading,
        alternatives: block.alternatives.slice(0, 8),
        region: block.bbox,
        blockDigest: digest,
      });
    }
  }
  return out;
}

export function applyClarificationAnswers(
  pages: PageDocument[],
  questions: ClarificationQuestion[],
  answers: ClarificationAnswer[]
): { updatedPages: PageDocument[]; changedCount: number; contextNotes: string[] } {
  if (answers.length > 32) {
    throw new Error('NG630: Too many clarification answers');
  }
  const seenIds = new Set<string>();
  const contextNotes: string[] = [];
  let changedCount = 0;

  const clonedPages: PageDocument[] = pages.map((p) => ({
    ...p,
    blocks: p.blocks.map((b) => ({
      ...b,
      rows: b.rows.map((r) => [...r]),
      merges: b.merges.map((m) => ({ ...m })),
      alternatives: [...b.alternatives],
    })),
    warnings: [...p.warnings],
  }));

  for (const a of answers) {
    if (seenIds.has(a.id)) {
      throw new Error('NG630: Duplicate clarification answer');
    }
    seenIds.add(a.id);
    const q = questions.find((item) => item.id === a.id);
    if (!q) {
      throw new Error('NG630: Unknown clarification question');
    }
    if (a.skip) continue;

    const targetPage = clonedPages.find((p) => p.page === q.page);
    if (!targetPage) throw new Error('NG630: Source page not found');
    const block = targetPage.blocks[q.blockIndex];
    if (!block) throw new Error('NG630: Source block changed');
    if (blockDigest(block) !== q.blockDigest) {
      throw new Error('NG630: Source block changed while answering; reopen Review');
    }

    if (a.replacement.trim().length > 0) {
      const clean = a.replacement.trim();
      if (q.field === 'meaning') {
        block.interpretation = clean;
      } else if (q.field === 'text') {
        block.text = clean;
      } else if (q.field.startsWith('cell:')) {
        const parts = q.field.slice(5).split(':').map(Number);
        if (
          parts.length !== 2 ||
          !block.rows[parts[0]] ||
          block.rows[parts[0]][parts[1]] === undefined
        ) {
          throw new Error('NG630: Table cell changed');
        }
        block.rows[parts[0]][parts[1]] = clean;
      }
      block.uncertain = blockPlain(block).includes('[illegible]');
      block.confidence = Math.max(block.confidence, 0.99);
      targetPage.edited = true;
      changedCount++;
    }

    if (a.context.trim().length > 0) {
      contextNotes.push(`Page ${q.page}, block ${q.blockIndex + 1}: ${a.context.trim()}`);
    }
  }

  return { updatedPages: clonedPages, changedCount, contextNotes };
}

// ---------------------------------------------------------------------------
// 4. Writer Profile Memory & Glyph Averaging (rust/src/writers.rs)
// ---------------------------------------------------------------------------

export function validIdentifier(id: string): boolean {
  return id.length > 0 && id.length <= 100 && /^[A-Za-z0-9_-]+$/.test(id);
}

export function writerCoverage(profile: WriterProfile): string {
  const chars: string[] = [];
  for (let c = 65; c <= 90; c++) chars.push(String.fromCharCode(c));
  for (let c = 97; c <= 122; c++) chars.push(String.fromCharCode(c));
  for (let c = 48; c <= 57; c++) chars.push(String.fromCharCode(c));

  return chars
    .filter((ch) => profile.samples.some((s) => s.glyph && s.label === ch))
    .join('');
}

export function synthesizeGlyphInkGrid(ch: string, variantSeed = 0): number[] {
  // Deterministic 16x20 grayscale ink bitmap (0=white, 255=ink) representing handwritten stroke patterns
  const W = 16;
  const H = 20;
  const grid = new Array<number>(W * H).fill(0);
  const code = ch.charCodeAt(0) || 65;
  const jitterX = (variantSeed % 3) - 1;

  const setInk = (x: number, y: number, val = 240) => {
    const xi = Math.round(x + jitterX * 0.3);
    const yi = Math.round(y);
    if (xi >= 1 && xi < W - 1 && yi >= 1 && yi < H - 1) {
      grid[yi * W + xi] = Math.max(grid[yi * W + xi], val);
      if (xi + 1 < W - 1) {
        grid[yi * W + (xi + 1)] = Math.max(
          grid[yi * W + (xi + 1)],
          Math.round(val * 0.72)
        );
      }
    }
  };

  const line = (x0: number, y0: number, x1: number, y1: number, steps = 18) => {
    for (let i = 0; i <= steps; i++) {
      const t = i / steps;
      setInk(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, 245);
    }
  };

  if (ch === 'r') {
    // Distinctive handwritten 'r' with a high looped shoulder
    line(4, 7, 4, 16);
    line(4, 10, 8, 7);
    line(8, 7, 12, 8);
    line(12, 8, 10, 11);
  } else if (ch === 'e' || ch === 'c') {
    line(11, 9, 6, 8);
    line(6, 8, 4, 12);
    line(4, 12, 6, 16);
    line(6, 16, 11, 15);
    if (ch === 'e') line(4, 12, 11, 11);
  } else if (ch === 'n' || ch === 'h') {
    line(4, ch === 'h' ? 3 : 8, 4, 16);
    line(4, 10, 8, 8);
    line(8, 8, 11, 10);
    line(11, 10, 11, 16);
  } else if (ch === 'u' || ch === 'w') {
    line(4, 8, 4, 15);
    line(4, 15, 8, 16);
    line(11, 8, 11, 16);
    if (ch === 'w') line(8, 11, 8, 16);
  } else if (ch === 'l' || ch === 'i' || ch === '1') {
    line(7, ch === 'i' ? 8 : 3, 7, 16);
    line(7, 16, 10, 15);
    if (ch === 'i') setInk(7, 5, 250);
  } else if (ch === 't' || ch === 'f') {
    line(7, 3, 7, 16);
    line(4, 8, 11, 8);
    if (ch === 't') line(7, 16, 10, 15);
  } else if (ch === 's' || ch === '2' || ch === '3') {
    line(5, 8, 10, 8);
    line(10, 8, 6, 12);
    line(6, 12, 11, 14);
    line(11, 14, 5, 16);
  } else if (ch === 'y') {
    line(4, 8, 7, 14);
    line(11, 8, 5, 18);
  } else {
    const topY = ch >= 'a' && ch <= 'z' ? 8 : 4;
    const botY = 'gjpq'.includes(ch) ? 18 : 16;
    line(4, topY, 4, botY);
    line(4, topY, 10, topY + 1);
    if (code % 2 === 0) line(4, Math.round((topY + botY) / 2), 9, Math.round((topY + botY) / 2));
    if (code % 3 === 0) line(10, topY + 2, 10, botY);
  }

  return grid;
}

export function averageGlyphGrid(profile: WriterProfile, ch: string): number[] {
  const matching = profile.samples
    .filter((s) => s.glyph && s.label === ch && s.inkGrid.length === 16 * 20)
    .slice(-32);

  if (matching.length === 0) {
    throw new Error(`NG620: No confirmed glyph samples for '${ch}'`);
  }

  const len = 16 * 20;
  const sums = new Array<number>(len).fill(0);
  for (const s of matching) {
    for (let i = 0; i < len; i++) {
      sums[i] += s.inkGrid[i];
    }
  }
  return sums.map((sum) => Math.round(sum / matching.length));
}

// ---------------------------------------------------------------------------
// 5. Model Validation & SHA256 Range Check (rust/src/models.rs)
// ---------------------------------------------------------------------------

export function validateReaderModel(m: ReaderModel): void {
  if (
    !validIdentifier(m.id) ||
    !validIdentifier(m.family) ||
    !m.name.trim() ||
    m.name.length > 160 ||
    !['ocr', 'vision'].includes(m.role) ||
    !['structured', 'lighton', 'glm'].includes(m.prompt) ||
    m.rank < 0 ||
    m.rank > 1000 ||
    m.resident_mib < 256 ||
    m.resident_mib > 200000
  ) {
    throw new Error(
      'NG650: Invalid reader ID, family, role, prompt protocol or memory estimate'
    );
  }
  if (m.endpoint !== undefined && m.endpoint.trim() !== '') {
    const ep = m.endpoint.trim();
    if (
      !ep.startsWith('http://127.0.0.1:') &&
      !ep.startsWith('http://localhost:') &&
      !ep.startsWith('http://[::1]:')
    ) {
      throw new Error(
        'NG203: Use a literal loopback endpoint. Redirects and remote conversion servers are blocked.'
      );
    }
    if (!m.alias.trim()) {
      throw new Error('NG650: Enter the exact local server model alias');
    }
  }
  if (m.hub) {
    const parts = m.hub.repository.split('/');
    if (parts.length !== 2 || parts.some((p) => !p || p.includes(' '))) {
      throw new Error('NG650: Use a Hugging Face owner/repository');
    }
    if (!m.hub.weights.endsWith('.gguf') || !m.hub.projector.endsWith('.gguf')) {
      throw new Error('NG650: Use exact repository-relative GGUF filenames');
    }
  }
}

export function verifyContentRange(header: string, start: number, total: number): boolean {
  if (!header.startsWith('bytes ')) return false;
  const rest = header.slice(6);
  const [range, sizeStr] = rest.split('/');
  if (!range || !sizeStr) return false;
  const [firstStr, lastStr] = range.split('-');
  const first = Number(firstStr);
  const last = Number(lastStr);
  const size = Number(sizeStr);
  return (
    Number.isInteger(first) &&
    Number.isInteger(last) &&
    Number.isInteger(size) &&
    first === start &&
    size === total &&
    last >= start &&
    last < total
  );
}
