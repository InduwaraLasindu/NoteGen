import React, { useState } from 'react';
import { AppearanceSettings, Backend } from '../utils/engine';
import { DEFAULT_APPEARANCE } from '../utils/demoData';
import { RotateCcw, Sliders, X } from 'lucide-react';

interface ComputeSettings {
  backend: Backend;
  cpu_threads: number;
  gpu_layers: number;
  context: number;
  image_tokens: number;
  batch: number;
  micro_batch: number;
  max_tokens: number;
  reserve_ram_mib: number;
  reserve_vram_mib: number;
  request_timeout_secs: number;
  deskew: boolean;
  auto_rotate: boolean;
  preserve_figures: boolean;
  keep_page_breaks: boolean;
}

interface AdvancedModalProps {
  open: boolean;
  appearance: AppearanceSettings;
  compute: ComputeSettings;
  onClose: () => void;
  onUpdateAppearance: (next: AppearanceSettings) => void;
  onUpdateCompute: (next: ComputeSettings) => void;
}

const ACCENT_PRESETS: Array<{ name: string; rgb: [number, number, number] }> = [
  { name: 'NoteGen Mint', rgb: [100, 226, 204] },
  { name: 'Cobalt Blue', rgb: [56, 189, 248] },
  { name: 'Amber Gold', rgb: [245, 158, 11] },
  { name: 'Emerald Leaf', rgb: [52, 211, 153] },
];

export const AdvancedModal: React.FC<AdvancedModalProps> = ({
  open,
  appearance,
  compute,
  onClose,
  onUpdateAppearance,
  onUpdateCompute,
}) => {
  const [tab, setTab] = useState<'compute' | 'recognition' | 'appearance' | 'json'>(
    'appearance'
  );
  const [rawJson, setRawJson] = useState('');
  const [jsonError, setJsonError] = useState('');

  if (!open) return null;

  const openJsonTab = () => {
    setRawJson(JSON.stringify({ compute, ui: appearance }, null, 2));
    setJsonError('');
    setTab('json');
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Advanced settings"
      className="fixed inset-0 z-40 flex items-center justify-center bg-black/75 p-4"
    >
      <div className="w-full max-w-2xl rounded-xl bg-white dark:bg-[#131B2A] border border-slate-200 dark:border-slate-800 shadow-2xl overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-slate-200 dark:border-slate-800">
          <div className="flex items-center gap-2">
            <Sliders className="w-4 h-4 text-teal-500" />
            <h2 className="text-base font-semibold text-slate-900 dark:text-slate-100">
              Advanced
            </h2>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close advanced settings"
            className="p-1.5 rounded-lg text-slate-500 hover:text-slate-900 dark:hover:text-white"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Tabs */}
        <div className="flex items-center gap-1.5 px-6 py-3 border-b border-slate-200 dark:border-slate-800 bg-slate-50 dark:bg-slate-900/40">
          {[
            { id: 'appearance', label: 'Appearance & Motion' },
            { id: 'compute', label: 'Compute & Memory' },
            { id: 'recognition', label: 'Scan & Word Export' },
          ].map((t) => (
            <button
              key={t.id}
              type="button"
              onClick={() => setTab(t.id as typeof tab)}
              className={`px-3 py-1.5 text-xs font-medium rounded-lg transition-colors whitespace-nowrap shrink-0 ${
                tab === t.id
                  ? 'bg-teal-500/20 text-teal-700 dark:text-teal-300 border border-teal-500/40'
                  : 'text-slate-600 dark:text-slate-400 hover:text-slate-900 dark:hover:text-white'
              }`}
            >
              {t.label}
            </button>
          ))}
          <button
            type="button"
            onClick={openJsonTab}
            className={`px-3 py-1.5 text-xs font-medium rounded-lg transition-colors whitespace-nowrap shrink-0 ${
              tab === 'json'
                ? 'bg-teal-500/20 text-teal-700 dark:text-teal-300 border border-teal-500/40'
                : 'text-slate-600 dark:text-slate-400 hover:text-slate-900 dark:hover:text-white'
            }`}
          >
            All parameters (JSON)
          </button>
        </div>

        {/* Body */}
        <div className="p-6 max-h-[70vh] overflow-y-auto space-y-5">
          {tab === 'appearance' && (
            <div className="space-y-4">
              <div className="flex flex-wrap items-center justify-between gap-4">
                <div>
                  <div className="text-sm font-medium text-slate-900 dark:text-slate-100">
                    Theme Mode
                  </div>
                  <div className="text-xs text-slate-500 dark:text-slate-400">
                    Switch between dark slate studio and high-contrast light mode
                  </div>
                </div>
                <div className="flex items-center gap-1 p-1 rounded-lg bg-slate-100 dark:bg-slate-800">
                  <button
                    type="button"
                    onClick={() =>
                      onUpdateAppearance({ ...appearance, dark: true })
                    }
                    className={`px-3 py-1.5 text-xs font-medium rounded-md transition-colors ${
                      appearance.dark
                        ? 'bg-slate-900 text-white shadow-sm'
                        : 'text-slate-600 dark:text-slate-400'
                    }`}
                  >
                    Dark Studio
                  </button>
                  <button
                    type="button"
                    onClick={() =>
                      onUpdateAppearance({ ...appearance, dark: false })
                    }
                    className={`px-3 py-1.5 text-xs font-medium rounded-md transition-colors ${
                      !appearance.dark
                        ? 'bg-white text-slate-900 shadow-sm'
                        : 'text-slate-600 dark:text-slate-400'
                    }`}
                  >
                    Daylight Paper
                  </button>
                </div>
              </div>

              <div>
                <div className="text-xs font-medium text-slate-700 dark:text-slate-300 mb-2">
                  Accent Palette
                </div>
                <div className="flex flex-wrap items-center gap-2">
                  {ACCENT_PRESETS.map((preset) => {
                    const active =
                      appearance.accent[0] === preset.rgb[0] &&
                      appearance.accent[1] === preset.rgb[1];
                    return (
                      <button
                        key={preset.name}
                        type="button"
                        onClick={() =>
                          onUpdateAppearance({
                            ...appearance,
                            accent: preset.rgb,
                          })
                        }
                        className={`px-3 py-1.5 text-xs font-medium rounded-lg border flex items-center gap-2 transition-colors ${
                          active
                            ? 'border-teal-400 bg-teal-500/15 text-slate-900 dark:text-white'
                            : 'border-slate-200 dark:border-slate-700 text-slate-600 dark:text-slate-300'
                        }`}
                      >
                        <span
                          className="w-3 h-3 rounded-full inline-block"
                          style={{
                            backgroundColor: `rgb(${preset.rgb.join(',')})`,
                          }}
                        />
                        {preset.name}
                      </button>
                    );
                  })}
                </div>
              </div>

              <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 pt-2">
                <div>
                  <label
                    htmlFor="motion-speed-slider"
                    className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1"
                  >
                    Motion speed ({appearance.motion.toFixed(1)}x)
                  </label>
                  <input
                    id="motion-speed-slider"
                    type="range"
                    min="0.2"
                    max="2.5"
                    step="0.1"
                    value={appearance.motion}
                    onChange={(e) =>
                      onUpdateAppearance({
                        ...appearance,
                        motion: Number(e.target.value),
                      })
                    }
                    className="w-full accent-teal-500"
                  />
                </div>
                <div>
                  <label
                    htmlFor="orbit-density-slider"
                    className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1"
                  >
                    Orbit particle density ({appearance.ambient_density})
                  </label>
                  <input
                    id="orbit-density-slider"
                    type="range"
                    min="0"
                    max="6"
                    step="1"
                    value={appearance.ambient_density}
                    onChange={(e) =>
                      onUpdateAppearance({
                        ...appearance,
                        ambient_density: Number(e.target.value),
                      })
                    }
                    className="w-full accent-teal-500"
                  />
                </div>
              </div>

              <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5 pt-2">
                <label className="flex items-center gap-2 text-xs text-slate-700 dark:text-slate-300 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={appearance.scan_animation}
                    onChange={(e) =>
                      onUpdateAppearance({
                        ...appearance,
                        scan_animation: e.target.checked,
                      })
                    }
                    className="rounded border-slate-400 text-teal-500"
                  />
                  <span>Scanning page beam graphic</span>
                </label>
                <label className="flex items-center gap-2 text-xs text-slate-700 dark:text-slate-300 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={appearance.particles}
                    onChange={(e) =>
                      onUpdateAppearance({
                        ...appearance,
                        particles: e.target.checked,
                      })
                    }
                    className="rounded border-slate-400 text-teal-500"
                  />
                  <span>Orbiting accents</span>
                </label>
                <label className="flex items-center gap-2 text-xs text-slate-700 dark:text-slate-300 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={appearance.glow}
                    onChange={(e) =>
                      onUpdateAppearance({
                        ...appearance,
                        glow: e.target.checked,
                      })
                    }
                    className="rounded border-slate-400 text-teal-500"
                  />
                  <span>Subtle surface glow</span>
                </label>
                <label className="flex items-center gap-2 text-xs text-slate-700 dark:text-slate-300 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={appearance.reduced_motion}
                    onChange={(e) =>
                      onUpdateAppearance({
                        ...appearance,
                        reduced_motion: e.target.checked,
                      })
                    }
                    className="rounded border-slate-400 text-teal-500"
                  />
                  <span>Reduced motion (freeze decorative graphics)</span>
                </label>
              </div>
            </div>
          )}

          {tab === 'compute' && (
            <div className="space-y-4">
              <p className="text-xs text-slate-500 dark:text-slate-400">
                0 means adaptive. GPU layers -1 means full offload when the model fits. Memory reserves keep the system and interface responsive.
              </p>
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                <div>
                  <label
                    htmlFor="compute-backend-select"
                    className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1"
                  >
                    Hardware Backend
                  </label>
                  <select
                    id="compute-backend-select"
                    value={compute.backend}
                    onChange={(e) =>
                      onUpdateCompute({
                        ...compute,
                        backend: e.target.value as Backend,
                      })
                    }
                    className="w-full rounded-lg bg-white dark:bg-slate-900 border border-slate-300 dark:border-slate-700 px-3 py-2 text-xs text-slate-900 dark:text-slate-100"
                  >
                    <option value="auto">Auto (CUDA / Vulkan / CPU)</option>
                    <option value="cuda">NVIDIA CUDA</option>
                    <option value="vulkan">Vulkan</option>
                    <option value="cpu">CPU Only</option>
                  </select>
                </div>
                <div>
                  <label
                    htmlFor="compute-threads-input"
                    className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1"
                  >
                    CPU Threads
                  </label>
                  <input
                    id="compute-threads-input"
                    type="number"
                    value={compute.cpu_threads}
                    onChange={(e) =>
                      onUpdateCompute({
                        ...compute,
                        cpu_threads: Number(e.target.value),
                      })
                    }
                    className="w-full rounded-lg bg-white dark:bg-slate-900 border border-slate-300 dark:border-slate-700 px-3 py-2 text-xs font-mono text-slate-900 dark:text-slate-100"
                  />
                </div>
                <div>
                  <label
                    htmlFor="compute-ram-input"
                    className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1"
                  >
                    RAM Reserve (MiB)
                  </label>
                  <input
                    id="compute-ram-input"
                    type="number"
                    value={compute.reserve_ram_mib}
                    onChange={(e) =>
                      onUpdateCompute({
                        ...compute,
                        reserve_ram_mib: Number(e.target.value),
                      })
                    }
                    className="w-full rounded-lg bg-white dark:bg-slate-900 border border-slate-300 dark:border-slate-700 px-3 py-2 text-xs font-mono text-slate-900 dark:text-slate-100"
                  />
                </div>
                <div>
                  <label
                    htmlFor="compute-vram-input"
                    className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1"
                  >
                    VRAM Reserve (MiB)
                  </label>
                  <input
                    id="compute-vram-input"
                    type="number"
                    value={compute.reserve_vram_mib}
                    onChange={(e) =>
                      onUpdateCompute({
                        ...compute,
                        reserve_vram_mib: Number(e.target.value),
                      })
                    }
                    className="w-full rounded-lg bg-white dark:bg-slate-900 border border-slate-300 dark:border-slate-700 px-3 py-2 text-xs font-mono text-slate-900 dark:text-slate-100"
                  />
                </div>
              </div>
            </div>
          )}

          {tab === 'recognition' && (
            <div className="space-y-3">
              <label className="flex items-center gap-2.5 text-xs text-slate-700 dark:text-slate-300 cursor-pointer">
                <input
                  type="checkbox"
                  checked={compute.deskew}
                  onChange={(e) =>
                    onUpdateCompute({ ...compute, deskew: e.target.checked })
                  }
                  className="rounded border-slate-400 text-teal-500"
                />
                <span>Small-angle deskew &amp; contrast normalization</span>
              </label>
              <label className="flex items-center gap-2.5 text-xs text-slate-700 dark:text-slate-300 cursor-pointer">
                <input
                  type="checkbox"
                  checked={compute.auto_rotate}
                  onChange={(e) =>
                    onUpdateCompute({
                      ...compute,
                      auto_rotate: e.target.checked,
                    })
                  }
                  className="rounded border-slate-400 text-teal-500"
                />
                <span>Automatic page orientation detection</span>
              </label>
              <label className="flex items-center gap-2.5 text-xs text-slate-700 dark:text-slate-300 cursor-pointer">
                <input
                  type="checkbox"
                  checked={compute.preserve_figures}
                  onChange={(e) =>
                    onUpdateCompute({
                      ...compute,
                      preserve_figures: e.target.checked,
                    })
                  }
                  className="rounded border-slate-400 text-teal-500"
                />
                <span>Preserve cropped diagrams &amp; contextual interpretations in Word</span>
              </label>
              <label className="flex items-center gap-2.5 text-xs text-slate-700 dark:text-slate-300 cursor-pointer">
                <input
                  type="checkbox"
                  checked={compute.keep_page_breaks}
                  onChange={(e) =>
                    onUpdateCompute({
                      ...compute,
                      keep_page_breaks: e.target.checked,
                    })
                  }
                  className="rounded border-slate-400 text-teal-500"
                />
                <span>Keep source page breaks in exported Word document</span>
              </label>
            </div>
          )}

          {tab === 'json' && (
            <div className="space-y-3">
              <textarea
                rows={10}
                aria-label="Settings JSON editor"
                value={rawJson}
                onChange={(e) => setRawJson(e.target.value)}
                className="w-full rounded-lg bg-slate-950 text-slate-200 font-mono text-xs p-3 border border-slate-800"
              />
              {jsonError && <p className="text-xs text-red-400">{jsonError}</p>}
              <button
                type="button"
                onClick={() => {
                  try {
                    const parsed = JSON.parse(rawJson);
                    if (parsed.ui) onUpdateAppearance(parsed.ui);
                    if (parsed.compute) onUpdateCompute(parsed.compute);
                    setJsonError('');
                    onClose();
                  } catch (e) {
                    setJsonError(
                      e instanceof Error ? e.message : 'Invalid JSON'
                    );
                  }
                }}
                className="px-4 py-2 text-xs font-semibold rounded-lg bg-teal-500 text-slate-950"
              >
                Apply validated JSON
              </button>
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between px-6 py-3.5 bg-slate-50 dark:bg-slate-900/60 border-t border-slate-200 dark:border-slate-800">
          <button
            type="button"
            onClick={() => onUpdateAppearance(DEFAULT_APPEARANCE)}
            className="px-3 py-1.5 text-xs font-medium rounded-lg border border-slate-300 dark:border-slate-700 text-slate-700 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 flex items-center gap-1.5"
          >
            <RotateCcw className="w-3.5 h-3.5" />
            Reset defaults
          </button>
          <button
            type="button"
            onClick={onClose}
            className="px-4 py-2 text-xs font-semibold rounded-lg bg-teal-500 text-slate-950 hover:bg-teal-400"
          >
            Done
          </button>
        </div>
      </div>
    </div>
  );
};
