import { Backend, Fault, Settings, validateSettings } from './types';

export interface GpuDevice {
  index: number;
  name: string;
  total_mib: number;
  free_mib: number;
  backend: 'cuda' | 'vulkan';
}

export interface Hardware {
  logical_cores: number;
  ram_mib: number;
  free_ram_mib: number;
  gpus: GpuDevice[];
}

export interface ModelHub {
  repository: string;
  weights: string;
  projector: string;
}

export interface ModelSpec {
  id: string;
  name: string;
  family: string;
  role: 'ocr' | 'vision';
  prompt: 'lighton' | 'glm' | 'structured';
  resident_mib: number;
  weights_mib: number;
  rank: number;
  source: string;
  released: string;
  weights: string;
  projector: string;
  sha256: string;
  endpoint?: string;
  alias: string;
  revision: string;
  hub?: ModelHub;
}

export interface Plan {
  backend: Backend;
  gpu_index: number | null;
  ram_budget_mib: number;
  vram_budget_mib: number;
  threads: number;
  dpi: number;
  image_edge: number;
  context: number;
  image_tokens: number;
  batch: number;
  micro_batch: number;
  gpu_layers: number;
  models: ModelSpec[];
  notes: string[];
}

export const CURATED_CATALOG: ModelSpec[] = [
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
    weights: 'models/lighton3-1b-q8_0.gguf',
    projector: 'models/lighton3-1b-mmproj.gguf',
    sha256: 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
    alias: 'lighton3-1b',
    revision: 'a19c84d20f9e',
    hub: {
      repository: 'lightonai/LightOnOCR-3-1B',
      weights: 'lighton3-1b-q8_0.gguf',
      projector: 'lighton3-1b-mmproj.gguf',
    },
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
    weights: 'models/glm-ocr-0.9b-q8_0.gguf',
    projector: 'models/glm-ocr-mmproj.gguf',
    sha256: '8f434346648f6b96df89dda901c5176b10a6d83961dd3c1ac88b59b2dc327aa4',
    alias: 'glm-ocr',
    revision: '7c441e82b11a',
    hub: {
      repository: 'ggml-org/GLM-OCR-GGUF',
      weights: 'glm-ocr-0.9b-q8_0.gguf',
      projector: 'glm-ocr-mmproj.gguf',
    },
  },
  {
    id: 'lighton3-4b',
    name: 'LightOnOCR-3 4B Q8',
    family: 'lighton3',
    role: 'ocr',
    prompt: 'lighton',
    resident_mib: 8500,
    weights_mib: 6200,
    rank: 96,
    source: 'https://huggingface.co/lightonai/LightOnOCR-3-4B',
    released: '2026-10-08',
    weights: 'models/lighton3-4b-q8_0.gguf',
    projector: 'models/lighton3-4b-mmproj.gguf',
    sha256: '4b227777d4dd1fc61c6f884f48641d02b4d121d3fd328cb08b5531fcacdabf8a',
    alias: 'lighton3-4b',
    revision: '99d04a11e330',
    hub: {
      repository: 'lightonai/LightOnOCR-3-4B',
      weights: 'lighton3-4b-q8_0.gguf',
      projector: 'lighton3-4b-mmproj.gguf',
    },
  },
  {
    id: 'qwen35-2b',
    name: 'Qwen3.5 2B Q4',
    family: 'qwen35',
    role: 'vision',
    prompt: 'structured',
    resident_mib: 2800,
    weights_mib: 2000,
    rank: 60,
    source: 'https://huggingface.co/Qwen/Qwen3.5-2B',
    released: '2026-03-02',
    weights: 'models/qwen3.5-2b-q4_k_m.gguf',
    projector: 'models/qwen3.5-2b-mmproj.gguf',
    sha256: 'ef2d127de37b942baad06145e54b0c619a1f22327b2ebbcfbec78f5564afe39d',
    alias: 'qwen35-2b',
    revision: '3f881c20b902',
    hub: {
      repository: 'Qwen/Qwen3.5-2B',
      weights: 'qwen3.5-2b-q4_k_m.gguf',
      projector: 'qwen3.5-2b-mmproj.gguf',
    },
  },
  {
    id: 'qwen35-4b',
    name: 'Qwen3.5 4B Q5',
    family: 'qwen35',
    role: 'vision',
    prompt: 'structured',
    resident_mib: 6000,
    weights_mib: 4000,
    rank: 72,
    source: 'https://huggingface.co/Qwen/Qwen3.5-4B',
    released: '2026-03-02',
    weights: 'models/qwen3.5-4b-q5_k_m.gguf',
    projector: 'models/qwen3.5-4b-mmproj.gguf',
    sha256: 'e7f6c011776e8db7cd330b54174fd76f7d0216b612387a5ffcfb81e6f0919683',
    alias: 'qwen35-4b',
    revision: '5b190d88411f',
    hub: {
      repository: 'Qwen/Qwen3.5-4B',
      weights: 'qwen3.5-4b-q5_k_m.gguf',
      projector: 'qwen3.5-4b-mmproj.gguf',
    },
  },
  {
    id: 'qwen35-9b',
    name: 'Qwen3.5 9B Q5',
    family: 'qwen35',
    role: 'vision',
    prompt: 'structured',
    resident_mib: 11800,
    weights_mib: 8500,
    rank: 88,
    source: 'https://huggingface.co/Qwen/Qwen3.5-9B',
    released: '2026-03-02',
    weights: 'models/qwen3.5-9b-q5_k_m.gguf',
    projector: 'models/qwen3.5-9b-mmproj.gguf',
    sha256: '7902699be42c8a8e46fbbb4501726517e86b22c56a189f7625a6da49081b2451',
    alias: 'qwen35-9b',
    revision: 'c0293847165a',
    hub: {
      repository: 'Qwen/Qwen3.5-9B',
      weights: 'qwen3.5-9b-q5_k_m.gguf',
      projector: 'qwen3.5-9b-mmproj.gguf',
    },
  },
  {
    id: 'qwen38-27b',
    name: 'Qwen3.8 27B Q4',
    family: 'qwen38',
    role: 'vision',
    prompt: 'structured',
    resident_mib: 25000,
    weights_mib: 19500,
    rank: 99,
    source: 'https://huggingface.co/Qwen/Qwen3.8-27B',
    released: '2026-08-14',
    weights: 'models/qwen3.8-27b-q4_k_m.gguf',
    projector: 'models/qwen3.8-27b-mmproj.gguf',
    sha256: '2c624232cdd221771294dfbb310aca000a0df6ac8b66b696d90ef06fdefb64a3',
    alias: 'qwen38-27b',
    revision: 'f8172635490b',
    hub: {
      repository: 'Qwen/Qwen3.8-27B',
      weights: 'qwen3.8-27b-q4_k_m.gguf',
      projector: 'qwen3.8-27b-mmproj.gguf',
    },
  },
];

export function defaultHardware(): Hardware {
  return {
    logical_cores: 16,
    ram_mib: 32768,
    free_ram_mib: 24576,
    gpus: [
      {
        index: 0,
        name: 'NVIDIA GeForce RTX 4070 Ti SUPER',
        total_mib: 16376,
        free_mib: 14820,
        backend: 'cuda',
      },
    ],
  };
}

export function buildPlan(hw: Hardware, s: Settings, models: ModelSpec[]): Plan {
  validateSettings(s);
  const gpu =
    s.gpu_index !== null
      ? hw.gpus.find((g) => g.index === s.gpu_index)
      : [...hw.gpus].sort((a, b) => b.free_mib - a.free_mib)[0];

  const backend: Backend =
    s.backend === 'auto'
      ? gpu
        ? gpu.backend === 'vulkan'
          ? 'vulkan'
          : 'cuda'
        : 'cpu'
      : s.backend;

  if (s.require_acceleration && (backend === 'cpu' || !gpu)) {
    throw new Fault('NG200', 'Hardware acceleration requested but no compatible GPU was detected');
  }

  const ram_budget = Math.min(
    Math.floor(hw.free_ram_mib * s.ram_fraction),
    Math.max(0, hw.free_ram_mib - s.reserve_ram_mib)
  );

  if (ram_budget < 768) {
    throw new Fault('NG300', 'Insufficient free system RAM');
  }

  const vram_budget =
    backend !== 'cpu' && gpu
      ? Math.min(
          Math.floor(gpu.free_mib * s.vram_fraction),
          Math.max(0, gpu.free_mib - s.reserve_vram_mib)
        )
      : 0;

  const fitting = models
    .filter((m) => {
      const allowed = s.automatic_models || s.model_ids.includes(m.id);
      const accelOk = !s.require_acceleration || !m.endpoint;
      const ramOk = Boolean(m.endpoint) || m.weights_mib + 512 <= ram_budget;
      let vramOk = false;
      if (m.endpoint) {
        vramOk = true;
      } else if (backend === 'cpu') {
        vramOk = m.weights_mib <= 6500;
      } else {
        vramOk =
          m.resident_mib <= vram_budget ||
          (s.allow_cpu_offload && m.weights_mib + 2048 <= ram_budget);
      }
      return allowed && accelOk && ramOk && vramOk;
    })
    .sort((a, b) => b.rank - a.rank);

  const chosen: ModelSpec[] = [];
  if (s.preset !== 'fast') {
    const ocr = fitting.find((m) => m.role === 'ocr');
    if (ocr) chosen.push(ocr);
    const vision = fitting.find((m) => m.role === 'vision');
    if (vision) chosen.push(vision);
    if (s.preset === 'maximum') {
      const families = new Set(chosen.map((m) => m.family));
      const other = fitting.find((m) => !families.has(m.family));
      if (other) chosen.push(other);
    }
  }

  const tier =
    vram_budget <= 3500 ? 0 : vram_budget <= 7500 ? 1 : vram_budget <= 15500 ? 2 : 3;
  const high = s.preset === 'maximum';
  const choose = (value: number, auto: number) => (value === 0 ? auto : value);

  const notes: string[] = [
    'Memory estimates include weights, projector, context and workspace; runtime admission checks remain required.',
  ];
  if (chosen.length < 2 && s.preset !== 'fast') {
    notes.push(
      'Insufficient compatible model diversity. Results will be flagged for review; no fictitious ensemble agreement.'
    );
  }
  if (backend === 'cpu') {
    notes.push('CPU backend selected: inference uses native SIMD. No AI GPU acceleration is claimed.');
  }

  return {
    backend,
    gpu_index: backend === 'cpu' ? null : gpu ? gpu.index : null,
    ram_budget_mib: ram_budget,
    vram_budget_mib: vram_budget,
    threads:
      s.cpu_threads === 0
        ? Math.max(1, hw.logical_cores - 1)
        : Math.min(s.cpu_threads, Math.max(1, hw.logical_cores)),
    dpi: choose(s.dpi, high ? 400 : 300),
    image_edge: choose(s.image_edge, [1536, 2048, 2560, 3072][tier]),
    context: choose(s.context, tier === 0 ? 12288 : high ? 32768 : 16384),
    image_tokens: choose(s.image_tokens, [1536, 3072, 4096, 6144][tier]),
    batch: choose(s.batch, tier === 0 ? 256 : 1024),
    micro_batch: choose(s.micro_batch, [64, 128, 256, 512][tier]),
    gpu_layers: backend === 'cpu' ? 0 : s.gpu_layers < 0 ? 99 : s.gpu_layers,
    models: chosen,
    notes,
  };
}
