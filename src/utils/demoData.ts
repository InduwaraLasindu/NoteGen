import {
  AppearanceSettings,
  PageDocument,
  ReaderModel,
  WriterProfile,
  synthesizeGlyphInkGrid,
} from './engine';

export const DEFAULT_APPEARANCE: AppearanceSettings = {
  dark: true,
  accent: [100, 226, 204],
  scale: 1.0,
  motion: 1.0,
  scan_animation: true,
  intro_animation: true,
  hover_effects: true,
  transition_ms: 200,
  ambient_density: 3,
  idle_fps: 30,
  reduced_motion: false,
  particles: true,
  glow: true,
  rounding: 12,
  fps: 60,
  compact: false,
};

export const INITIAL_MODELS: ReaderModel[] = [
  {
    id: 'lighton3-1b',
    name: 'LightOnOCR-3 1B Q8',
    family: 'lighton3',
    role: 'ocr',
    prompt: 'lighton',
    resident_mib: 2300,
    weights_mib: 1500,
    rank: 82,
    source: 'https://huggingface.co/lightonai/LightOnOCR-3-1B',
    released: '2026-10-08',
    revision: '85b208e3ad87ac8d022067e2a491dbd0f9beb79f',
    sha256: '7d59f5dddd10897cd75871b8176f12bb0397a87375a3fd688a1bfa151f3eb0e6',
    alias: 'lighton3-1b',
    hub: {
      repository: 'richardyoung/LightOnOCR-3-1B-GGUF',
      weights: 'LightOnOCR-3-1B-Q8_0.gguf',
      projector: 'mmproj-LightOnOCR-3-1B-F16.gguf',
      available_revision: 'c94f1108e291aa45b022067e2a491dbd0f9b9912',
      available_sha256: '9f88e120ab44ac8d022067e2a491dbd0f9beb79f8176f12bb0397a87375a3fd6',
      available_bytes: 1458756832,
    },
    enabled: true,
  },
  {
    id: 'glm-ocr',
    name: 'GLM-OCR 0.9B Q8',
    family: 'glm',
    role: 'ocr',
    prompt: 'glm',
    resident_mib: 2400,
    weights_mib: 1800,
    rank: 80,
    source: 'https://huggingface.co/ggml-org/GLM-OCR-GGUF',
    released: '2026-01',
    revision: '65a42de1148dbed2297e922b5dbc7d9b70c36578',
    sha256: '45bc244a6446aff850521dc41f18bc8d7105ad5f0c2c8c28af04e7cc4f4d50b1',
    alias: 'glm-ocr',
    hub: {
      repository: 'ggml-org/GLM-OCR-GGUF',
      weights: 'GLM-OCR-Q8_0.gguf',
      projector: 'mmproj-GLM-OCR-Q8_0.gguf',
      available_revision: '65a42de1148dbed2297e922b5dbc7d9b70c36578',
      available_sha256: '45bc244a6446aff850521dc41f18bc8d7105ad5f0c2c8c28af04e7cc4f4d50b1',
      available_bytes: 1434837056,
    },
    enabled: true,
  },
  {
    id: 'qwen35-4b',
    name: 'Qwen3.5 4B Q5 Vision',
    family: 'qwen35',
    role: 'vision',
    prompt: 'structured',
    resident_mib: 6000,
    weights_mib: 4000,
    rank: 72,
    source: 'https://huggingface.co/Qwen/Qwen3.5-4B',
    released: '2026-03-02',
    revision: 'e87f176479d0855a907a41277aca2f8ee7a09523',
    sha256: '8814232b85594dcd46c50e5b8b29324a7efe9e746edbe8a3d1df3d3fce7aad39',
    alias: 'qwen35-4b',
    hub: {
      repository: 'unsloth/Qwen3.5-4B-GGUF',
      weights: 'Qwen3.5-4B-Q5_K_M.gguf',
      projector: 'mmproj-F16.gguf',
      available_revision: 'f19d884129d0855a907a41277aca2f8ee7a04410',
      available_sha256: '3109482b85594dcd46c50e5b8b29324a7efe9e746edbe8a3d1df3d3fce7aa901',
      available_bytes: 3816080224,
    },
    enabled: true,
  },
  {
    id: 'qwen38-27b',
    name: 'Qwen3.8 27B Q4 Specialist',
    family: 'qwen38',
    role: 'vision',
    prompt: 'structured',
    resident_mib: 25000,
    weights_mib: 19500,
    rank: 99,
    source: 'https://huggingface.co/Qwen/Qwen3.8-27B',
    released: '2026-08-14',
    revision: 'a316ea527fde32c12487be7574e2db5ac2ad2a7e',
    sha256: 'c600de0300ae8a0eb3a6c0b8b5561b8b96f16bd2c863c2a66c42de29d391a747',
    alias: 'qwen38-27b',
    hub: {
      repository: 'ggml-org/Qwen3.8-27B-GGUF',
      weights: 'Qwen3.8-27B-Q4_K_M.gguf',
      projector: 'mmproj-Qwen3.8-27B-Q8_0.gguf',
    },
    enabled: true,
  },
];

function createInitialSamplesForAda() {
  const letters = [
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'V',
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
    '0', '1', '2', '3', '4', '5', '8',
  ];
  const samples = letters.map((ch, idx) => ({
    id: `sample-ada-${ch.charCodeAt(0)}-v1`,
    label: ch,
    glyph: true,
    created: '2026-10-30T14:20:00Z',
    source: 'Fluid_Mechanics_Lecture_04.pdf page 1',
    inkGrid: synthesizeGlyphInkGrid(ch, 0),
    strokeTrait:
      ch === 'r'
        ? 'Looped right shoulder (often misread as v without profile)'
        : ch === 'q'
        ? 'Hooked descender tail'
        : idx % 2 === 0
        ? 'High x-height terminal stroke'
        : 'Consistent 14deg forward slant',
  }));

  // Add second sample for 'r' and 'a' to demonstrate multi-sample averaging
  samples.push({
    id: 'sample-ada-114-v2',
    label: 'r',
    glyph: true,
    created: '2026-10-31T09:15:00Z',
    source: 'Fluid_Mechanics_Lecture_04.pdf page 2',
    inkGrid: synthesizeGlyphInkGrid('r', 1),
    strokeTrait: 'Looped right shoulder (sample #2)',
  });

  // Add word-level samples
  samples.push(
    {
      id: 'sample-ada-word-bernoulli',
      label: 'Bernoulli',
      glyph: false,
      created: '2026-10-31T10:02:00Z',
      source: 'Fluid_Mechanics_Lecture_04.pdf page 1',
      inkGrid: synthesizeGlyphInkGrid('B', 0),
      strokeTrait: 'Connected cursive ligature on double-l',
    },
    {
      id: 'sample-ada-word-reynolds',
      label: 'Reynolds',
      glyph: false,
      created: '2026-10-31T10:12:00Z',
      source: 'Fluid_Mechanics_Lecture_04.pdf page 1',
      inkGrid: synthesizeGlyphInkGrid('R', 0),
      strokeTrait: 'Open loop on lowercase r and d',
    }
  );

  return samples;
}

export const INITIAL_WRITERS: WriterProfile[] = [
  {
    schema: 1,
    id: 'writer-ada-lovelace-01',
    name: 'Ada Lovelace',
    notes:
      'Lowercase "r" has a high looped shoulder that resembles "v". Lowercase "q" has an upward hooked tail. Equations use Q for volumetric flow rate.',
    revision: 14,
    samples: createInitialSamplesForAda(),
  },
  {
    schema: 1,
    id: 'writer-aris-thorne-02',
    name: 'Dr. Aris Thorne',
    notes:
      'Rapid clinical & lab shorthand. Crosses 7s and Zs with horizontal bar. Capital I has top and bottom serifs.',
    revision: 5,
    samples: ['A', 'B', 'C', 'E', 'I', 'R', 'S', 'T', 'a', 'e', 'i', 'o', 'r', 's', 't', '7'].map(
      (ch) => ({
        id: `sample-aris-${ch.charCodeAt(0)}`,
        label: ch,
        glyph: true,
        created: '2026-10-29T18:00:00Z',
        source: 'Lab_Bench_Synthesis_Notes.pdf page 1',
        inkGrid: synthesizeGlyphInkGrid(ch, 2),
        strokeTrait: 'Sharp angular downstroke',
      })
    ),
  },
];

export const INITIAL_PAGES: PageDocument[] = [
  {
    page: 1,
    title: 'Fluid_Mechanics_Lecture_04.pdf — Page 1',
    image_id: 'fluid-lecture-p1',
    agreement: 0.865,
    edited: false,
    reviewed: false,
    warnings: [
      'Independent readers disagree on handwritten word "turbulence / tuvbulence" in Block 2.',
      'Numeric disagreement in Table Row 2 Column 2: 1250 vs 1280. Check against source scan.',
      'AI diagram interpretations require source verification.',
    ],
    blocks: [
      {
        id: 'b-heading-1',
        kind: 'heading',
        text: 'Section 4.2: Siphon & Venturi Flow Balance',
        rows: [],
        header_rows: 0,
        merges: [],
        description: '',
        interpretation: '',
        bbox: { left: 0.075, top: 0.08, right: 0.82, bottom: 0.155 },
        confidence: 0.98,
        uncertain: false,
        alternatives: [
          'lighton3-1b: Section 4.2: Siphon & Venturi Flow Balance',
          'qwen35-4b: Section 4.2: Siphon & Venturi Flow Balance',
        ],
      },
      {
        id: 'b-text-2',
        kind: 'text',
        text: 'In steady incompressible flow between Tank A and Tank B, onset of [illegible] occurs when the local Reynolds number exceeds 2300 along the constricted throat.',
        rows: [],
        header_rows: 0,
        merges: [],
        description: '',
        interpretation: '',
        bbox: { left: 0.075, top: 0.175, right: 0.91, bottom: 0.35 },
        confidence: 0.76,
        uncertain: true,
        alternatives: [
          'lighton3-1b: ...onset of turbulence occurs when the local Reynolds number exceeds 2300...',
          'glm-ocr: ...onset of tuvbulence occurs when the local Reynolds number exceeds 2300...',
          'tesseract: ...onset of [illegible] occurs when the local Reynolds number exceeds 2300...',
        ],
      },
      {
        id: 'b-table-3',
        kind: 'table',
        text: '',
        rows: [
          ['Conduit Segment', 'Volumetric Flow Q (mL/s)', 'Static Head h (cm)'],
          ['Inlet Reservoir A', '1250', '48.5'],
          ['Venturi Throat B', '1250 [illegible]', '19.2'],
        ],
        header_rows: 1,
        merges: [],
        description: '',
        interpretation: '',
        bbox: { left: 0.06, top: 0.475, right: 0.94, bottom: 0.72 },
        confidence: 0.82,
        uncertain: true,
        alternatives: [
          'glm-ocr: Row 3 Col 2 = 1250',
          'qwen35-4b: Row 3 Col 2 = 1250 mL/s',
          'tesseract: Row 3 Col 2 = 1280',
        ],
      },
      {
        id: 'b-figure-4',
        kind: 'figure',
        text: '',
        rows: [],
        header_rows: 0,
        merges: [],
        description:
          'Hand-sketched two-tank siphon diagram with upper vessel A, crest tube C, constricted throat B, and piezometer columns showing pressure drop Δh.',
        interpretation:
          'Demonstrates Bernoulli head conservation where velocity head rises at throat B, reducing static piezometric head by Δh = 29.3 cm.',
        bbox: { left: 0.06, top: 0.735, right: 0.94, bottom: 0.975 },
        confidence: 0.84,
        uncertain: true,
        alternatives: [
          'qwen35-4b: Siphon elevation sketch linking Tank A and Tank B with piezometric head markers.',
        ],
      },
    ],
    candidates: [
      {
        model_id: 'lighton3-1b',
        family: 'lighton3',
        view: 'full',
        reliability: 0.89,
        plain_text:
          'Section 4.2: Siphon & Venturi Flow Balance In steady incompressible flow between Tank A and Tank B, onset of turbulence occurs when the local Reynolds number exceeds 2300 along the constricted throat. Conduit Segment Volumetric Flow Q (mL/s) Static Head h (cm) Inlet Reservoir A 1250 48.5 Venturi Throat B 1250 19.2',
        blocks: [],
      },
      {
        model_id: 'qwen35-4b',
        family: 'qwen35',
        view: 'full',
        reliability: 0.93,
        plain_text:
          'Section 4.2: Siphon & Venturi Flow Balance In steady incompressible flow between Tank A and Tank B, onset of turbulence occurs when the local Reynolds number exceeds 2300 along the constricted throat. Conduit Segment Volumetric Flow Q (mL/s) Static Head h (cm) Inlet Reservoir A 1250 48.5 Venturi Throat B 1250 19.2',
        blocks: [],
      },
      {
        model_id: 'glm-ocr',
        family: 'glm',
        view: 'full',
        reliability: 0.84,
        plain_text:
          'Section 4.2: Siphon & Venturi Flow Balance In steady incompressible flow between Tank A and Tank B, onset of tuvbulence occurs when the local Reynolds number exceeds 2300 along the constricted throat. Conduit Segment Volumetric Flow Q (mL/s) Static Head h (cm) Inlet Reservoir A 1250 48.5 Venturi Throat B 1250 19.2',
        blocks: [],
      },
      {
        model_id: 'tesseract',
        family: 'tesseract',
        view: 'full',
        reliability: 0.45,
        plain_text:
          'Section 4.2: Siphon & Venturi Flow Balance In steady incompressible flow between Tank A and Tank B, onset of [illegible] occurs when the local Reynolds number exceeds 2300 along the constricted throat. Conduit Segment Volumetric Flow Q (mL/s) Static Head h (cm) Inlet Reservoir A 1250 48.5 Venturi Throat B 1280 19.2',
        blocks: [],
      },
    ],
  },
];

export interface RustAuditFix {
  id: string;
  file: string;
  severity: 'High' | 'Medium';
  title: string;
  cause: string;
  fixSummary: string;
  verifiedBy: string;
}

export const RUST_AUDIT_FIXES: RustAuditFix[] = [
  {
    id: 'NG-AUDIT-01',
    file: 'rust/src/writers.rs (normalize)',
    severity: 'High',
    title: 'False Dark-Background Inversion on Tight Handwritten Glyph Crops',
    cause:
      'normalize() only averaged top and bottom scanline pixels (y=0, y=h-1) and inverted whenever edge_mean < 128. Tightly cropped ascenders/descenders touching top/bottom borders pulled edge_mean below 128 on white paper, inverting ink and paper.',
    fixSummary:
      'Sample all 4 perimeter edges (top, bottom, left, right) with zero-division guard and require edge_mean < 96 before inverting.',
    verifiedBy: 'Chunk 2 — Writer Profile Normalization & Averaging Gate',
  },
  {
    id: 'NG-AUDIT-02',
    file: 'rust/src/consensus.rs (select)',
    severity: 'High',
    title: 'False-Positive Numeric Disagreement on Trailing Sentence Punctuation',
    cause:
      'The numbers() closure kept "." and "," for decimals but did not strip trailing sentence periods (e.g. "1250." vs "1250"), triggering false numeric conflict warnings when one reader included a trailing period.',
    fixSummary:
      'Added .trim_matches([\'.\', \',\']) after outer non-alphanumeric stripping so "1250." and "1250" normalize identically while preserving internal decimals like "48.5".',
    verifiedBy: 'Chunk 1 — Independent Family Medoid & Numeric Consensus Gate',
  },
  {
    id: 'NG-AUDIT-03',
    file: 'rust/src/assistance.rs (questions)',
    severity: 'High',
    title: 'Stale Block Answer Prefix Suppressing Re-Read Clarification Prompts',
    cause:
      'questions() checked a.id.starts_with("p{n}-b{index}-"), which suppressed new clarification questions on a block even after its content and block_digest changed during a contextual re-read.',
    fixSummary:
      'Compute the full block digest ID first (p{n}-b{index}-{digest[..10]}) and match exact question IDs so modified blocks are re-prompted.',
    verifiedBy: 'Chunk 4 — Clarification Digest Guard & Context Re-Read Gate',
  },
  {
    id: 'NG-AUDIT-04',
    file: 'rust/src/fontpack.rs (ttf)',
    severity: 'Medium',
    title: 'TrueType Binary Table Checksum & Unsorted Glyph Mapping Ordering',
    cause:
      'cmap Format 4 requires character code mappings in strictly ascending order; unsorted glyph arrays produce corrupt TrueType character lookups.',
    fixSummary:
      'Enforced ascending Unicode codepoint sort before building glyf, loca, hmtx, and cmap Format 4 tables, and verified 0xB1B0AFBA head checkSumAdjustment.',
    verifiedBy: 'Chunk 3 — TrueType (.ttf) Binary Checksum & Contour Gate',
  },
];
