export type Preset = 'fast' | 'balanced' | 'accuracy' | 'maximum';
export type Backend = 'auto' | 'cuda' | 'vulkan' | 'cpu';

export interface Appearance {
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
  background: [number, number, number];
  autosave: boolean;
}

export interface WriterOptions {
  root: string;
  id: string | null;
  use_examples: boolean;
}

export interface WebSource {
  title: string;
  url: string;
  excerpt: string;
  retrieved: string;
}

export interface AssistanceOptions {
  ask_questions: boolean;
  max_questions: number;
  web_lookup: boolean;
  glossary: string;
  sources: WebSource[];
}

export interface Settings {
  preset: Preset;
  automatic_models: boolean;
  model_ids: string[];
  handwriting: boolean;
  languages: string;
  page_range: string;
  dpi: number;
  max_pixels: number;
  image_edge: number;
  tile_edge: number;
  tile_overlap: number;
  crop_retries: number;
  consensus_min: number;
  agreement_threshold: number;
  review_threshold: number;
  backend: Backend;
  gpu_index: number | null;
  ram_fraction: number;
  vram_fraction: number;
  reserve_ram_mib: number;
  reserve_vram_mib: number;
  cpu_threads: number;
  context: number;
  max_tokens: number;
  batch: number;
  micro_batch: number;
  image_tokens: number;
  gpu_layers: number;
  flash_attention: boolean;
  projector_on_cpu: boolean;
  kv_cache: string;
  temperature: number;
  top_p: number;
  seed: number;
  request_timeout_secs: number;
  allow_cpu_offload: boolean;
  require_acceleration: boolean;
  tesseract: string;
  pdfium: string;
  server: string;
  models_dir: string;
  endpoint: string;
  external_server: boolean;
  subject: string;
  writer: WriterOptions;
  assistance: AssistanceOptions;
  deskew: boolean;
  auto_rotate: boolean;
  preserve_figures: boolean;
  keep_page_breaks: boolean;
  font: string;
  font_size: number;
  ui: Appearance;
}

export interface BoxRect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export interface Merge {
  row: number;
  col: number;
  rowspan: number;
  colspan: number;
}

export type BlockKind = 'text' | 'heading' | 'list' | 'table' | 'figure' | 'equation';

export interface Block {
  kind: BlockKind;
  text: string;
  box?: BoxRect;
  confidence?: number;
  rows: string[][];
  merges: Merge[];
  header_rows: number;
  description: string;
  interpretation: string;
  uncertain: boolean;
  alternatives: string[];
}

export interface PageContent {
  blocks: Block[];
  warnings: string[];
  summary: string;
}

export interface Candidate {
  region?: BoxRect;
  model_id: string;
  family: string;
  view: string;
  content: PageContent;
  reliability: number;
}

export interface Page {
  edited: boolean;
  page: number;
  image: string;
  content: PageContent;
  candidates: Candidate[];
  source_warnings: string[];
  agreement: number;
  reviewed: boolean;
  seconds: number;
}

export interface Question {
  id: string;
  page: number;
  block: number;
  field: string;
  prompt: string;
  reading: string;
  alternatives: string[];
  region?: BoxRect;
  image: string;
  block_digest: string;
}

export interface Answer {
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
  image: string;
  created: string;
  source: string;
}

export interface WriterProfile {
  schema: number;
  id: string;
  name: string;
  notes: string;
  revision: number;
  samples: WriterSample[];
}

export interface FontOptions {
  family: string;
  spacing: number;
  threshold: number;
}

export interface ProjectData {
  id: string;
  name: string;
  input: string;
  output: string;
  settings: Settings;
  pages: Page[];
  notes: string[];
  answers: Answer[];
}

export type JobStatus = 'Pending' | 'Running' | 'Complete' | 'Failed' | 'Cancelled';

export interface Job {
  id: string;
  input: string;
  status: JobStatus;
  output?: string;
  projectId?: string;
  detail: string;
  totalPages?: number;
}

export interface ActivityEvent {
  timestamp: string;
  stage: string;
  message: string;
  page: number;
  done: number;
  total: number;
  code: string;
  warning: boolean;
}

export class Fault extends Error {
  code: string;
  detail: string;

  constructor(code: string, detail: string) {
    super(`${code}: ${detail}`);
    this.code = code;
    this.detail = detail;
    this.name = 'Fault';
  }

  remedy(): string {
    switch (this.code) {
      case 'NG100':
        return 'Choose a readable, unlocked PDF or valid page range.';
      case 'NG102':
        return 'Check Advanced settings and restore defaults for invalid values.';
      case 'NG110':
        return 'Install Tesseract and the chosen language data, or configure a local vision model.';
      case 'NG200':
        return 'Configure a compatible llama-server executable and local model/projector files.';
      case 'NG201':
        return 'Review preserved candidates; the model returned invalid or truncated output.';
      case 'NG202':
      case 'NG300':
        return 'Free RAM/VRAM or reduce model, image/context size. Resume saved work.';
      case 'NG203':
        return 'Use a literal loopback endpoint. Redirects and remote conversion servers are blocked.';
      case 'NG400':
        return 'Choose a writable output folder; close any open Word file and check disk space.';
      case 'NG401':
        return 'Choose a new output filename; existing files are protected.';
      case 'NG500':
        return 'Restore matching input/settings/models or start a new project.';
      case 'NG610':
        return 'Choose a valid writer and a tight crop containing visible handwriting. Label only confirmed examples.';
      case 'NG620':
        return 'Label individual English letters first, then export to a new font-pack filename.';
      case 'NG630':
        return 'Reopen the source block, confirm its text, and save a shorter clarification.';
      case 'NG640':
        return 'Enable public lookup and enter a topic; check the connection or continue offline.';
      case 'NG650':
        return 'Check model metadata, available disk space and the publisher revision. The active version is preserved.';
      case 'NG900':
        return 'Completed work is saved. Resume the same input and settings.';
      default:
        return 'Open the local activity log for the stage and error code.';
    }
  }
}

export function defaultAppearance(): Appearance {
  return {
    dark: true,
    accent: [100, 226, 204],
    scale: 1.0,
    motion: 1.0,
    scan_animation: true,
    intro_animation: true,
    hover_effects: true,
    transition_ms: 220,
    ambient_density: 3,
    idle_fps: 30,
    reduced_motion: false,
    particles: true,
    glow: true,
    rounding: 16,
    fps: 60,
    compact: false,
    background: [12, 17, 28],
    autosave: true,
  };
}

export function defaultSettings(): Settings {
  return {
    preset: 'accuracy',
    automatic_models: true,
    model_ids: [],
    handwriting: false,
    languages: 'eng',
    page_range: '',
    dpi: 0,
    max_pixels: 20_000_000,
    image_edge: 0,
    tile_edge: 1536,
    tile_overlap: 0.12,
    crop_retries: 8,
    consensus_min: 2,
    agreement_threshold: 0.92,
    review_threshold: 0.90,
    backend: 'auto',
    gpu_index: null,
    ram_fraction: 0.85,
    vram_fraction: 0.92,
    reserve_ram_mib: 1024,
    reserve_vram_mib: 512,
    cpu_threads: 0,
    context: 0,
    max_tokens: 8192,
    batch: 0,
    micro_batch: 0,
    image_tokens: 0,
    gpu_layers: -1,
    flash_attention: true,
    projector_on_cpu: false,
    kv_cache: 'q8_0',
    temperature: 0.1,
    top_p: 0.9,
    seed: 42,
    request_timeout_secs: 180,
    allow_cpu_offload: false,
    require_acceleration: false,
    tesseract: 'tesseract',
    pdfium: 'runtime',
    server: 'runtime/llama-server.exe',
    models_dir: 'models',
    endpoint: 'http://127.0.0.1:18745',
    external_server: false,
    subject: '',
    writer: {
      root: 'user-data/writers',
      id: null,
      use_examples: true,
    },
    assistance: {
      ask_questions: true,
      max_questions: 8,
      web_lookup: false,
      glossary: '',
      sources: [],
    },
    deskew: true,
    auto_rotate: true,
    preserve_figures: true,
    keep_page_breaks: true,
    font: 'Calibri',
    font_size: 11.0,
    ui: defaultAppearance(),
  };
}

export function validWriterId(id: string): boolean {
  return id.length > 0 && id.length <= 100 && /^[A-Za-z0-9_-]+$/.test(id);
}

export function validateLocalEndpoint(endpoint: string): void {
  let url: URL;
  try {
    url = new URL(endpoint);
  } catch {
    throw new Fault('NG203', 'Invalid local endpoint');
  }
  const host = url.hostname.replace(/^\[|\]$/g, '');
  const isLoopback =
    host === '127.0.0.1' ||
    host === '::1' ||
    /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(host);
  if (!isLoopback) {
    throw new Fault('NG203', 'Use a literal loopback IP');
  }
  if (
    url.protocol !== 'http:' ||
    url.username !== '' ||
    url.password !== '' ||
    url.search !== '' ||
    url.hash !== '' ||
    url.pathname !== '/'
  ) {
    throw new Fault('NG203', 'Remote or ambiguous endpoint blocked');
  }
}

export function validateSettings(s: Settings): void {
  const bad = (msg: string): never => {
    throw new Fault('NG102', msg);
  };
  if (s.handwriting && s.preset === 'fast') {
    bad('Handwriting requires Balanced, Accuracy or Maximum Accuracy. Fast is printed OCR only.');
  }
  if (s.dpi !== 0 && (s.dpi < 100 || s.dpi > 600)) {
    bad('DPI must be 0 (auto) or 100–600');
  }
  if (s.image_edge !== 0 && (s.image_edge < 512 || s.image_edge > 8192)) {
    bad('Image edge must be auto or 512–8192');
  }
  if (s.max_pixels < 1_000_000 || s.max_pixels > 40_000_000) {
    bad('Image memory limit outside supported range');
  }
  if (
    s.ram_fraction < 0.1 ||
    s.ram_fraction > 0.98 ||
    s.vram_fraction < 0.1 ||
    s.vram_fraction > 0.98
  ) {
    bad('Memory fractions must be 0.1–0.98');
  }
  if (
    s.tile_overlap < 0.0 ||
    s.tile_overlap > 0.4 ||
    s.tile_edge < 512 ||
    s.tile_edge > 4096 ||
    s.crop_retries > 64 ||
    s.crop_retries < 0
  ) {
    bad('Invalid crop settings');
  }
  if (
    s.agreement_threshold < 0.5 ||
    s.agreement_threshold > 1.0 ||
    s.review_threshold < 0.0 ||
    s.review_threshold > 1.0
  ) {
    bad('Invalid review/consensus threshold');
  }
  if (s.context !== 0 && (s.context < 4096 || s.context > 131072)) {
    bad('Context must be auto or 4096–131072');
  }
  if (
    s.max_tokens < 256 ||
    s.max_tokens > 32768 ||
    s.cpu_threads > 512 ||
    s.cpu_threads < 0 ||
    s.batch > 8192 ||
    s.batch < 0 ||
    s.micro_batch > 2048 ||
    s.micro_batch < 0 ||
    s.image_tokens > 16384 ||
    s.image_tokens < 0
  ) {
    bad('Invalid inference limits');
  }
  if (
    s.consensus_min < 2 ||
    s.consensus_min > 8 ||
    s.gpu_layers < -1 ||
    s.temperature < 0.0 ||
    s.temperature > 1.5 ||
    s.top_p < 0.0 ||
    s.top_p > 1.0
  ) {
    bad('Invalid inference/consensus configuration');
  }
  if (!['f16', 'q8_0', 'q4_0'].includes(s.kv_cache)) {
    bad('Unsupported KV cache precision');
  }
  if (
    s.request_timeout_secs < 5 ||
    s.request_timeout_secs > 900 ||
    s.ui.scale < 0.7 ||
    s.ui.scale > 2.0 ||
    s.ui.motion < 0.0 ||
    s.ui.motion > 3.0 ||
    s.ui.fps < 15 ||
    s.ui.fps > 144 ||
    s.ui.idle_fps < 1 ||
    s.ui.idle_fps > 60 ||
    s.ui.transition_ms > 1000 ||
    s.ui.transition_ms < 0 ||
    s.ui.ambient_density > 8 ||
    s.ui.ambient_density < 0
  ) {
    bad('Invalid timeout or UI settings');
  }
  const langParts = s.languages.split('+');
  if (langParts.some((part) => !part || !/^[A-Za-z0-9_]+$/.test(part))) {
    bad('Invalid Tesseract language codes');
  }
  if (s.subject.length > 8000 || s.font.length > 100 || s.font_size < 6.0 || s.font_size > 32.0) {
    bad('Invalid document settings');
  }
  if (
    s.model_ids.length > 32 ||
    s.model_ids.some((id) => !id || id.length > 100 || !/^[A-Za-z0-9_-]+$/.test(id))
  ) {
    bad('Invalid model allowlist');
  }
  if (
    s.assistance.max_questions > 32 ||
    s.assistance.max_questions < 0 ||
    s.assistance.glossary.length > 8000 ||
    s.assistance.sources.length > 12
  ) {
    bad('Clarification/context limits exceeded');
  }
  if (s.writer.id !== null && !validWriterId(s.writer.id)) {
    bad('Invalid writer profile ID');
  }
  validateLocalEndpoint(s.endpoint);
}

export function isBoxValid(b: BoxRect): boolean {
  return (
    [b.left, b.top, b.right, b.bottom].every((v) => Number.isFinite(v) && v >= 0 && v <= 1) &&
    b.left < b.right &&
    b.top < b.bottom
  );
}

export function blockPlain(b: Block): string {
  if (b.kind === 'table') {
    return b.rows.map((r) => r.join('\t')).join('\n');
  }
  return b.text;
}

export function pagePlain(content: PageContent): string {
  return content.blocks
    .filter((b) => b.kind !== 'figure')
    .map(blockPlain)
    .join('\n');
}

export function validateBlock(b: Block): void {
  const validKinds: BlockKind[] = ['text', 'heading', 'list', 'table', 'figure', 'equation'];
  if (!validKinds.includes(b.kind)) {
    throw new Fault('NG201', 'Unknown block kind');
  }
  if (
    b.text.length >= 120000 ||
    b.description.length >= 24000 ||
    b.interpretation.length >= 24000 ||
    b.alternatives.length > 64
  ) {
    throw new Fault('NG201', 'Oversized content');
  }
  if (b.box && !isBoxValid(b.box)) {
    throw new Fault('NG201', 'Invalid region box');
  }
  if (b.confidence !== undefined && (!Number.isFinite(b.confidence) || b.confidence < 0 || b.confidence > 1)) {
    throw new Fault('NG201', 'Invalid confidence');
  }
  if (b.kind === 'figure' && !b.box) {
    throw new Fault('NG201', 'Figure requires a source box');
  }
  if (b.kind === 'table') {
    const cols = b.rows[0]?.length ?? 0;
    if (
      b.rows.length < 1 ||
      b.rows.length > 100 ||
      cols < 1 ||
      cols > 40 ||
      b.rows.some((r) => r.length !== cols) ||
      b.header_rows > b.rows.length
    ) {
      throw new Fault('NG201', 'Invalid table dimensions');
    }
    const seen = new Set<string>();
    for (const m of b.merges) {
      if (
        m.rowspan <= 0 ||
        m.colspan <= 0 ||
        m.row + m.rowspan > b.rows.length ||
        m.col + m.colspan > cols
      ) {
        throw new Fault('NG201', 'Invalid table merge');
      }
      for (let r = m.row; r < m.row + m.rowspan; r++) {
        for (let c = m.col; c < m.col + m.colspan; c++) {
          const key = `${r}:${c}`;
          if (seen.has(key)) {
            throw new Fault('NG201', 'Overlapping table merges');
          }
          seen.add(key);
        }
      }
    }
  }
}

export function validatePageContent(content: PageContent): void {
  if (content.summary.length > 24000 || content.warnings.length > 200 || content.blocks.length > 500) {
    throw new Fault('NG201', 'Excessive page data');
  }
  for (const b of content.blocks) {
    validateBlock(b);
  }
}

export function parsePagesRange(spec: string, total: number): number[] {
  if (!spec.trim()) {
    return Array.from({ length: total }, (_, i) => i + 1);
  }
  const set = new Set<number>();
  for (const part of spec.split(',')) {
    const parts = part.trim().split('-');
    if (parts.length < 1 || parts.length > 2) {
      throw new Fault('NG100', 'Invalid page range');
    }
    const a = Number.parseInt(parts[0], 10);
    const b = Number.parseInt(parts[parts.length - 1], 10);
    if (Number.isNaN(a) || Number.isNaN(b) || a <= 0 || a > b || b > total) {
      throw new Fault('NG100', 'Page range outside document');
    }
    for (let p = a; p <= b; p++) {
      set.add(p);
    }
  }
  return Array.from(set).sort((x, y) => x - y);
}

export function evidenceLabel(page: Page): string {
  const families = new Set(page.candidates.map((c) => c.family)).size;
  if (families === 0) return 'No reader evidence';
  if (families === 1) return 'Single reader family · independent agreement unavailable';
  return `Agreement ${(page.agreement * 100).toFixed(1)}% · ${families} reader families · ${page.candidates.length} readings`;
}

export function pageNeedsReview(page: Page, threshold: number): boolean {
  return (
    !page.reviewed &&
    (page.agreement < 0.92 ||
      page.content.warnings.length > 0 ||
      page.source_warnings.length > 0 ||
      page.content.blocks.length === 0 ||
      page.content.blocks.some(
        (b) =>
          b.uncertain ||
          b.kind === 'figure' ||
          b.confidence === undefined ||
          b.confidence < threshold ||
          blockPlain(b).toLowerCase().includes('[illegible]')
      ))
  );
}
