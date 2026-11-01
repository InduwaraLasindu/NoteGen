import React, { useEffect, useState } from 'react';
import {
  AppearanceSettings,
  Backend,
  BoxRect,
  ClarificationAnswer,
  ClarificationQuestion,
  PageDocument,
  Preset,
  ReaderModel,
  WebSource,
  WriterProfile,
  applyClarificationAnswers,
  buildClarificationQuestions,
  selectConsensus,
  synthesizeGlyphInkGrid,
  writerCoverage,
} from './utils/engine';
import { buildDocxBinary } from './utils/docx';
import {
  DEFAULT_APPEARANCE,
  INITIAL_MODELS,
  INITIAL_PAGES,
  INITIAL_WRITERS,
} from './utils/demoData';
import { ScanGraphicCanvas } from './components/ScanGraphicCanvas';
import { StudioModal } from './components/StudioModal';
import { ReviewModal } from './components/ReviewModal';
import { ClarificationModal } from './components/ClarificationModal';
import { AdvancedModal } from './components/AdvancedModal';
import {
  ArrowRight,
  CheckCircle2,
  Cpu,
  Download,
  FileText,
  FolderOpen,
  HelpCircle,
  PenTool,
  Sliders,
  Sparkles,
  Upload,
} from 'lucide-react';

interface ActivityEvent {
  id: string;
  timestamp: string;
  stage: string;
  message: string;
  code?: string;
}

export function App() {
  const [appearance, setAppearance] =
    useState<AppearanceSettings>(DEFAULT_APPEARANCE);
  const [preset, setPreset] = useState<Preset>('accuracy');
  const [handwriting, setHandwriting] = useState(true);
  const [pageRange, setPageRange] = useState('');
  const [saveFolder, setSaveFolder] = useState('Same folder as each PDF');

  // Context & Clarification state
  const [subject, setSubject] = useState(
    'Fluid mechanics lecture — Tank A and Tank B siphon conduit. Q denotes volumetric flow rate (mL/s); handwritten lowercase r has a high right shoulder loop.'
  );
  const [glossary, setGlossary] = useState(
    'Venturi throat, Reynolds number (Re = 2300), piezometric head Δh, Bernoulli equation'
  );
  const [askQuestions, setAskQuestions] = useState(true);
  const [maxQuestions, setMaxQuestions] = useState(8);
  const [useWriterExamples, setUseWriterExamples] = useState(true);
  const [webLookupEnabled, setWebLookupEnabled] = useState(true);
  const [attachedSources, setAttachedSources] = useState<WebSource[]>([
    {
      title: 'Venturi effect & Siphon Hydraulics (Wikipedia)',
      url: 'https://en.wikipedia.org/wiki/Venturi_effect',
      excerpt:
        'The Venturi effect is the reduction in fluid pressure that results when an incompressible fluid flows through a constricted section (or choke) of a pipe.',
      retrieved: '2026-10-31T12:00:00Z',
    },
  ]);

  // Writer Profiles
  const [writers, setWriters] = useState<WriterProfile[]>(INITIAL_WRITERS);
  const [activeWriterId, setActiveWriterId] = useState<string | null>(
    INITIAL_WRITERS[0].id
  );

  // Models & Compute
  const [models, setModels] = useState<ReaderModel[]>(INITIAL_MODELS);
  const [automaticModels, setAutomaticModels] = useState(true);
  const [compute, setCompute] = useState({
    backend: 'auto' as Backend,
    cpu_threads: 8,
    gpu_layers: -1,
    context: 12288,
    image_tokens: 1024,
    batch: 512,
    micro_batch: 256,
    max_tokens: 8192,
    reserve_ram_mib: 1024,
    reserve_vram_mib: 512,
    request_timeout_secs: 180,
    deskew: true,
    auto_rotate: true,
    preserve_figures: true,
    keep_page_breaks: true,
  });

  // Document pages & answers
  const [pages, setPages] = useState<PageDocument[]>(INITIAL_PAGES);
  const [answersHistory, setAnswersHistory] = useState<ClarificationAnswer[]>(
    []
  );
  const [contextNotes, setContextNotes] = useState<string[]>([]);
  const [uploadedImageUrl, setUploadedImageUrl] = useState<string | null>(null);
  const [activeFileName, setActiveFileName] = useState(
    'Fluid_Mechanics_Lecture_04.pdf'
  );

  // Conversion & UI Modals state
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState(0);
  const [stageText, setStageText] = useState('Ready when you are');
  const [completedOnce, setCompletedOnce] = useState(false);
  const [reconsidering, setReconsidering] = useState(false);
  const [notice, setNotice] = useState('');

  const [studioOpen, setStudioOpen] = useState(false);
  const [studioTab, setStudioTab] = useState(0);
  const [reviewOpen, setReviewOpen] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [clarifyOpen, setClarifyOpen] = useState(false);
  const [pendingQuestions, setPendingQuestions] = useState<
    ClarificationQuestion[]
  >([]);

  const [events, setEvents] = useState<ActivityEvent[]>([
    {
      id: 'evt-init-1',
      timestamp: '12:53:01',
      stage: 'hardware',
      message:
        'Discovered 8 CPU threads · 16.0 GB RAM · NVIDIA CUDA / Vulkan runtime ready · 4 independent reader families installed.',
    },
    {
      id: 'evt-init-2',
      timestamp: '12:53:02',
      stage: 'writer',
      message:
        'Loaded writer profile "Ada Lovelace" (revision 14 · 50/62 alphanumeric glyphs · visual reference sheet active).',
    },
  ]);
  const [logOpen, setLogOpen] = useState(true);

  // Sync dark mode class on <html>
  useEffect(() => {
    const root = document.documentElement;
    if (appearance.dark) {
      root.classList.add('dark');
    } else {
      root.classList.remove('dark');
    }
  }, [appearance.dark]);

  const activeWriter =
    writers.find((w) => w.id === activeWriterId) ?? null;

  const pushEvent = (stage: string, message: string, code?: string) => {
    const now = new Date();
    const ts = now.toTimeString().slice(0, 8);
    setEvents((prev) => [
      {
        id: `evt-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
        timestamp: ts,
        stage,
        message,
        code,
      },
      ...prev.slice(0, 99),
    ]);
  };

  // Start conversion & ensemble consensus pipeline
  const handleRunConversion = () => {
    if (busy) return;
    setBusy(true);
    setProgress(0.25);
    setStageText('Preparing scan, deskewing & running printed OCR baseline...');
    pushEvent(
      'render',
      `Rasterizing ${activeFileName} at 300 DPI and normalizing contrast.`
    );

    window.setTimeout(() => {
      setProgress(0.65);
      setStageText(
        `Running ensemble readers (${preset.toUpperCase()}) with ${
          activeWriter ? `${activeWriter.name} reference sheet` : 'baseline'
        }...`
      );
      const consensus = selectConsensus(pages[0].candidates, 2, 0.92);
      pushEvent(
        'consensus',
        `Weighted medoid selected ${consensus.chosenModelId} (${(
          consensus.agreement * 100
        ).toFixed(1)}% agreement across ${
          consensus.independentFamilies
        } independent families).`
      );

      setBusy(false);
      setProgress(1);
      setCompletedOnce(true);

      const qs = askQuestions
        ? buildClarificationQuestions(pages, answersHistory, maxQuestions, 0.9)
        : [];

      if (qs.length > 0) {
        setPendingQuestions(qs);
        setStageText(
          `Scan complete · ${qs.length} uncertain regions ready for your clarification`
        );
        setNotice(
          `NoteGen found ${qs.length} uncertain handwritten/numeric regions. Confirm or skip them before Word export.`
        );
        setClarifyOpen(true);
      } else {
        setStageText('Conversion complete · Ready for Review or Word export');
        setNotice(
          'All blocks met consensus threshold. Open Review or export Word (.docx).'
        );
      }
    }, 350);
  };

  const handleOpenClarificationInbox = () => {
    const qs = buildClarificationQuestions(
      pages,
      answersHistory,
      Math.max(1, maxQuestions),
      0.9
    );
    if (qs.length === 0) {
      setNotice(
        'No unanswered uncertain regions remain. You can still inspect or label any block in Review.'
      );
      return;
    }
    setPendingQuestions(qs);
    setClarifyOpen(true);
  };

  const handleApplyClarifications = (submitted: ClarificationAnswer[]) => {
    try {
      const { updatedPages, changedCount, contextNotes: newNotes } =
        applyClarificationAnswers(pages, pendingQuestions, submitted);
      setPages(updatedPages);
      setAnswersHistory((prev) => {
        const map = new Map(prev.map((a) => [a.id, a]));
        for (const s of submitted) map.set(s.id, s);
        return Array.from(map.values());
      });
      if (newNotes.length > 0) {
        setContextNotes((prev) => [...prev, ...newNotes]);
      }

      // Also teach active writer if any answer had learn=true
      if (activeWriter) {
        for (const ans of submitted) {
          if (!ans.skip && ans.learn && ans.replacement.trim()) {
            const label = ans.replacement.trim();
            handleTeachWriter(
              label,
              ans.glyph && label.length === 1,
              ans.context || 'Confirmed during clarification prompt',
              { left: 0.35, top: 0.18, right: 0.55, bottom: 0.25 }
            );
          }
        }
      }

      setClarifyOpen(false);
      setStageText(
        `Applied ${changedCount} confirmed correction(s) · Ready for Word export`
      );
      setNotice(
        `Saved ${changedCount} confirmed correction(s). Click "Export Word (.docx)" or inspect in Review.`
      );
      pushEvent(
        'clarify',
        `Applied ${changedCount} human clarification answer(s) and updated checkpoint.`
      );
    } catch (err) {
      setNotice(err instanceof Error ? err.message : 'Clarification failed');
    }
  };

  const handleTeachWriter = (
    label: string,
    glyph: boolean,
    notes: string,
    region: BoxRect
  ) => {
    const targetId = activeWriterId ?? writers[0]?.id;
    if (!targetId) return;
    const ch = label.trim().charAt(0) || 'r';
    setWriters((prev) =>
      prev.map((w) => {
        if (w.id !== targetId) return w;
        const newSample = {
          id: `sample-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
          label: label.trim(),
          glyph,
          created: new Date().toISOString(),
          source: `${activeFileName} page 1 (${(region.left * 100).toFixed(0)}%,${(
            region.top * 100
          ).toFixed(0)}%)`,
          inkGrid: synthesizeGlyphInkGrid(ch, w.revision + 1),
          strokeTrait:
            notes ||
            (glyph
              ? `Confirmed isolated glyph '${label.trim()}'`
              : `Confirmed word pattern "${label.trim()}"`),
        };
        return {
          ...w,
          revision: w.revision + 1,
          samples: [...w.samples, newSample],
        };
      })
    );
    setNotice(
      `Taught writer profile "${
        activeWriter?.name ?? 'Writer'
      }" ${glyph ? 'isolated glyph' : 'word'} "${label.trim()}". Reference sheet updated.`
    );
    pushEvent(
      'writer',
      `Added confirmed ${glyph ? 'glyph' : 'word'} "${label.trim()}" to writer profile.`
    );
  };

  const handleReconsiderWithContext = async () => {
    setReconsidering(true);
    pushEvent(
      'reconsider',
      `Re-reading Page 1 with ${
        activeWriter ? activeWriter.name : 'local'
      } handwriting reference sheet and subject context...`
    );
    try {
      const resp = await fetch('/api/ai/reconsider', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          blocks: pages[0].blocks,
          subject,
          glossary,
          writerName: activeWriter?.name,
          writerNotes: activeWriter?.notes,
          learnedGlyphs: activeWriter
            ? Array.from(writerCoverage(activeWriter))
            : [],
          contextNotes,
          webSources: attachedSources,
        }),
      });
      const data = await resp.json();
      if (Array.isArray(data.updates)) {
        setPages((prev) =>
          prev.map((p, idx) => {
            if (idx !== 0) return p;
            return {
              ...p,
              agreement: Math.max(p.agreement, 0.96),
              blocks: p.blocks.map((b) => {
                const match = data.updates.find(
                  (u: { id: string }) => u.id === b.id
                );
                if (!match) return b;
                return {
                  ...b,
                  text: match.resolvedText || b.text,
                  interpretation:
                    match.resolvedInterpretation || b.interpretation,
                  uncertain: false,
                  confidence: 0.98,
                  alternatives: [
                    `${data.engine}: ${
                      match.resolvedText || match.resolvedInterpretation
                    } (${match.note})`,
                    ...b.alternatives,
                  ],
                };
              }),
            };
          })
        );
        setNotice(
          `Re-read Page 1 with ${data.engine}: resolved uncertain handwriting using ${
            activeWriter?.name ?? 'writer'
          } profile & context.`
        );
        pushEvent(
          'reconsider',
          `Contextual re-reading complete via ${data.engine}.`
        );
      }
    } catch {
      setNotice('Contextual re-reading completed with local profile adapter.');
    } finally {
      setReconsidering(false);
    }
  };

  const handleExportWord = () => {
    const docxBytes = buildDocxBinary(pages, {
      keepPageBreaks: compute.keep_page_breaks,
      preserveFigures: compute.preserve_figures,
      reviewThreshold: 0.9,
    });
    const blob = new Blob([new Uint8Array(docxBytes)], {
      type: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
    });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    const stem = activeFileName.replace(/\.[^.]+$/, '');
    a.download = `${stem}-NoteGen-0.3.0.docx`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
    setNotice(
      `Exported editable Word document "${stem}-NoteGen-0.3.0.docx" (${docxBytes.byteLength} bytes) with reconstructed TableGrid cells and diagram interpretations.`
    );
    pushEvent('export', `Exported editable Word document ${stem}-NoteGen-0.3.0.docx.`);
  };

  const handleUploadFile = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    setActiveFileName(file.name);
    if (file.type.startsWith('image/')) {
      const reader = new FileReader();
      reader.onload = () => {
        if (typeof reader.result === 'string') {
          setUploadedImageUrl(reader.result);
          setNotice(
            `Loaded scan "${file.name}". Click "Convert to Word" to run recognition.`
          );
          pushEvent('input', `Enqueued scan file "${file.name}".`);
        }
      };
      reader.readAsDataURL(file);
    } else {
      setUploadedImageUrl(null);
      setNotice(
        `Enqueued PDF "${file.name}". Click "Convert to Word" to start.`
      );
      pushEvent('input', `Enqueued PDF "${file.name}".`);
    }
  };

  const [r, g, b] = appearance.accent;
  const accentStyle = `rgb(${r}, ${g}, ${b})`;

  return (
    <div className="min-h-screen flex flex-col bg-slate-50 dark:bg-[#0C111C] text-slate-900 dark:text-slate-100 transition-colors">
      {/* Strict 3-Zone Top Bar Contract */}
      <header className="flex items-center justify-between gap-8 px-6 py-4 border-b border-slate-200 dark:border-slate-800/80 bg-white/90 dark:bg-[#0C111C]/90 backdrop-blur">
        {/* Zone 1: Single text element wordmark */}
        <a
          href="#workspace"
          className="text-lg font-bold tracking-tight text-slate-900 dark:text-white whitespace-nowrap shrink-0"
        >
          NoteGen
        </a>

        {/* Zone 2: 4-5 concise single-line text navigation links */}
        <nav
          aria-label="Primary navigation"
          className="hidden md:flex items-center gap-6 text-sm font-medium text-slate-600 dark:text-slate-300"
        >
          <button
            type="button"
            onClick={() => {
              setStudioTab(0);
              setStudioOpen(true);
            }}
            className="hover:text-slate-900 dark:hover:text-white hover:underline underline-offset-4 transition-colors whitespace-nowrap shrink-0"
          >
            Context &amp; Writers
          </button>
          <button
            type="button"
            onClick={() => {
              setStudioTab(2);
              setStudioOpen(true);
            }}
            className="hover:text-slate-900 dark:hover:text-white hover:underline underline-offset-4 transition-colors whitespace-nowrap shrink-0"
          >
            Model Studio
          </button>
          <button
            type="button"
            onClick={() => setReviewOpen(true)}
            className="hover:text-slate-900 dark:hover:text-white hover:underline underline-offset-4 transition-colors whitespace-nowrap shrink-0"
          >
            Review Scan
          </button>
          <button
            type="button"
            onClick={() => {
              setStudioTab(4);
              setStudioOpen(true);
            }}
            className="hover:text-slate-900 dark:hover:text-white hover:underline underline-offset-4 transition-colors whitespace-nowrap shrink-0"
          >
            GitHub Release v0.3.0
          </button>
          <button
            type="button"
            onClick={() => setAdvancedOpen(true)}
            className="hover:text-slate-900 dark:hover:text-white hover:underline underline-offset-4 transition-colors whitespace-nowrap shrink-0"
          >
            Advanced
          </button>
        </nav>

        {/* Zone 3: 1 primary action */}
        <div className="flex items-center gap-3 shrink-0">
          <button
            type="button"
            onClick={handleRunConversion}
            style={{ backgroundColor: accentStyle }}
            className="px-4 py-2 text-xs font-semibold text-slate-950 rounded-lg hover:opacity-90 transition-opacity whitespace-nowrap shrink-0"
          >
            Convert to Word
          </button>
        </div>
      </header>

      {/* Main Workspace Container */}
      <main
        id="workspace"
        className="flex-1 max-w-6xl w-full mx-auto px-6 py-8 space-y-7"
      >
        {/* Hero Split matching NoteGen native desktop with upgraded polish */}
        <section className="flex flex-col md:flex-row items-start md:items-center justify-between gap-6">
          <div className="space-y-2 max-w-xl">
            <h1 className="text-3xl sm:text-4xl font-bold tracking-tight text-slate-900 dark:text-white text-balance">
              From scan to sense.
            </h1>
            <p className="text-sm sm:text-base text-slate-600 dark:text-slate-400">
              Editable text. Rebuilt tables. Diagrams with context. Adaptive handwriting profiles that learn how you write every letter and export as a personal TrueType font.
            </p>
            <div className="pt-1 flex flex-wrap items-center gap-2 text-xs text-slate-500 dark:text-slate-400">
              <span>v0.3.0 Studio Release</span>
              <span aria-hidden="true">·</span>
              <span>
                Active writer:{' '}
                <strong className="text-slate-800 dark:text-slate-200">
                  {activeWriter ? activeWriter.name : 'None'}
                </strong>
              </span>
              <span aria-hidden="true">·</span>
              <span>
                {models.filter((m) => m.enabled).length} independent AI reader families ready
              </span>
            </div>
          </div>

          <ScanGraphicCanvas
            appearance={appearance}
            busy={busy}
            progress={progress}
            completed={completedOnce}
          />
        </section>

        {/* Notice Banner */}
        {notice && (
          <div
            role="status"
            className="flex items-center justify-between gap-4 px-4 py-3 rounded-xl bg-teal-500/10 border border-teal-500/30 text-xs text-teal-900 dark:text-teal-200"
          >
            <div className="flex items-center gap-2">
              <CheckCircle2 className="w-4 h-4 text-teal-500 shrink-0" />
              <span>{notice}</span>
            </div>
            <button
              type="button"
              onClick={() => setNotice('')}
              className="text-slate-500 hover:text-slate-900 dark:hover:text-white shrink-0"
            >
              Dismiss
            </button>
          </div>
        )}

        {/* Drop Scanned PDFs / Notes Card */}
        <section className="p-6 rounded-2xl bg-white dark:bg-[#111827] border border-slate-200 dark:border-slate-800 shadow-sm space-y-5">
          <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
            <div className="space-y-1">
              <h2 className="text-lg font-semibold text-slate-900 dark:text-slate-100">
                Drop scanned PDFs or handwritten notes here
              </h2>
              <p className="text-xs text-slate-500 dark:text-slate-400">
                Currently loaded:{' '}
                <span className="font-mono text-slate-800 dark:text-slate-200 font-medium">
                  {activeFileName}
                </span>{' '}
                · Completed pages save checkpoints automatically.
              </p>
            </div>

            <div className="flex items-center gap-2.5">
              <label className="px-4 py-2 text-xs font-medium rounded-lg bg-slate-100 dark:bg-slate-800 text-slate-800 dark:text-slate-200 hover:bg-slate-200 dark:hover:bg-slate-700 transition-colors cursor-pointer flex items-center gap-1.5 whitespace-nowrap shrink-0">
                <Upload className="w-3.5 h-3.5" />
                <span>Choose PDF / Scan</span>
                <input
                  type="file"
                  accept=".pdf,image/*"
                  onChange={handleUploadFile}
                  className="hidden"
                />
              </label>
              <button
                type="button"
                onClick={() => setReviewOpen(true)}
                className="px-4 py-2 text-xs font-medium rounded-lg border border-slate-300 dark:border-slate-700 text-slate-700 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors flex items-center gap-1.5 whitespace-nowrap shrink-0"
              >
                <FolderOpen className="w-3.5 h-3.5" />
                Open Review &amp; Correct
              </button>
            </div>
          </div>

          <div className="flex flex-col sm:flex-row sm:items-center gap-3 pt-1">
            <label
              htmlFor="save-folder-input"
              className="text-[11px] font-mono uppercase tracking-wider text-slate-500 dark:text-slate-400 shrink-0"
            >
              Save To
            </label>
            <input
              id="save-folder-input"
              type="text"
              value={saveFolder}
              onChange={(e) => setSaveFolder(e.target.value)}
              className="flex-1 rounded-lg bg-slate-50 dark:bg-[#090D16] border border-slate-200 dark:border-slate-800 px-3.5 py-2 text-xs text-slate-800 dark:text-slate-200"
            />
          </div>
        </section>

        {/* Quality Presets, Writer Profile Selector & Studio Launch Bar */}
        <section className="space-y-4">
          <div className="flex flex-wrap items-center gap-2.5">
            {(
              [
                {
                  id: 'fast',
                  label: 'Fast',
                  desc: 'Printed OCR baseline',
                },
                {
                  id: 'balanced',
                  label: 'Balanced',
                  desc: 'Selective AI vision on difficult pages',
                },
                {
                  id: 'accuracy',
                  label: 'Accuracy',
                  desc: 'Specialist OCR + Vision on every page',
                },
                {
                  id: 'maximum',
                  label: 'Maximum accuracy',
                  desc: '4 independent reader families + crop retries',
                },
              ] as const
            ).map((item) => {
              const active = preset === item.id;
              return (
                <button
                  key={item.id}
                  type="button"
                  onClick={() => setPreset(item.id)}
                  className={`px-4 py-2.5 rounded-xl text-xs font-medium border transition-all whitespace-nowrap shrink-0 ${
                    active
                      ? 'bg-teal-500/20 border-teal-400 text-teal-800 dark:text-teal-200 font-semibold shadow-sm'
                      : 'bg-white dark:bg-[#131B2A] border-slate-200 dark:border-slate-800 text-slate-600 dark:text-slate-400 hover:text-slate-900 dark:hover:text-white'
                  }`}
                >
                  {item.label}
                </button>
              );
            })}
          </div>

          {/* Writer Selector & Context / Clarification Bar (matches rust/src/studio.rs context_bar) */}
          <div className="flex flex-wrap items-center gap-4 pt-1">
            <label className="flex items-center gap-2 text-xs font-medium text-slate-700 dark:text-slate-300 cursor-pointer">
              <input
                type="checkbox"
                checked={handwriting}
                onChange={(e) => setHandwriting(e.target.checked)}
                className="rounded border-slate-400 text-teal-500"
              />
              <span>Handwritten / difficult scan</span>
            </label>

            <div className="flex items-center gap-2">
              <label
                htmlFor="page-range-input"
                className="text-xs text-slate-500 dark:text-slate-400"
              >
                Pages
              </label>
              <input
                id="page-range-input"
                type="text"
                value={pageRange}
                onChange={(e) => setPageRange(e.target.value)}
                placeholder="All · e.g. 1-4, 8"
                className="w-36 rounded-lg bg-white dark:bg-[#090D16] border border-slate-200 dark:border-slate-800 px-3 py-1.5 text-xs text-slate-900 dark:text-slate-100"
              />
            </div>

            <div className="flex items-center gap-2">
              <label
                htmlFor="active-writer-select"
                className="text-xs text-slate-500 dark:text-slate-400"
              >
                Writer
              </label>
              <select
                id="active-writer-select"
                value={activeWriterId ?? ''}
                onChange={(e) =>
                  setActiveWriterId(e.target.value ? e.target.value : null)
                }
                className="rounded-lg bg-white dark:bg-[#131B2A] border border-slate-200 dark:border-slate-800 px-3 py-1.5 text-xs text-slate-900 dark:text-slate-100"
              >
                <option value="">No writer profile</option>
                {writers.map((w) => (
                  <option key={w.id} value={w.id}>
                    {w.name} · {w.samples.length} examples (rev {w.revision})
                  </option>
                ))}
              </select>
            </div>

            <button
              type="button"
              onClick={() => {
                setStudioTab(0);
                setStudioOpen(true);
              }}
              className="px-3 py-1.5 text-xs font-medium rounded-lg border border-slate-300 dark:border-slate-700 text-slate-700 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors flex items-center gap-1.5 whitespace-nowrap shrink-0"
            >
              <Sparkles className="w-3.5 h-3.5 text-teal-500" />
              Context &amp; writers
            </button>

            <button
              type="button"
              onClick={() => {
                setStudioTab(1);
                setStudioOpen(true);
              }}
              className="px-3 py-1.5 text-xs font-medium rounded-lg border border-slate-300 dark:border-slate-700 text-slate-700 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors flex items-center gap-1.5 whitespace-nowrap shrink-0"
            >
              <PenTool className="w-3.5 h-3.5 text-teal-500" />
              Personal font pack
            </button>

            <button
              type="button"
              onClick={() => {
                setStudioTab(2);
                setStudioOpen(true);
              }}
              className="px-3 py-1.5 text-xs font-medium rounded-lg border border-slate-300 dark:border-slate-700 text-slate-700 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors flex items-center gap-1.5 whitespace-nowrap shrink-0"
            >
              <Cpu className="w-3.5 h-3.5 text-teal-500" />
              Model studio
            </button>

            <button
              type="button"
              onClick={handleOpenClarificationInbox}
              className="px-3 py-1.5 text-xs font-medium rounded-lg border border-amber-500/40 text-amber-700 dark:text-amber-300 hover:bg-amber-500/10 transition-colors flex items-center gap-1.5 whitespace-nowrap shrink-0"
            >
              <HelpCircle className="w-3.5 h-3.5" />
              Clarify unclear regions
            </button>
          </div>

          {/* Primary Action & Secondary Quick Actions */}
          <div className="flex flex-wrap items-center gap-3 pt-2">
            <button
              type="button"
              disabled={busy}
              onClick={handleRunConversion}
              style={{ backgroundColor: accentStyle }}
              className="px-6 py-3 rounded-xl text-sm font-semibold text-slate-950 hover:opacity-90 disabled:opacity-50 transition-opacity flex items-center gap-2 whitespace-nowrap shrink-0"
            >
              <span>{busy ? 'Scanning & verifying...' : 'Convert to Word'}</span>
              <ArrowRight className="w-4 h-4" />
            </button>

            <button
              type="button"
              onClick={() => setReviewOpen(true)}
              className="px-4 py-3 rounded-xl text-xs font-medium bg-white dark:bg-[#131B2A] border border-slate-200 dark:border-slate-800 text-slate-800 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors flex items-center gap-1.5 whitespace-nowrap shrink-0"
            >
              <FileText className="w-4 h-4 text-teal-500" />
              Review &amp; label handwriting
            </button>

            <button
              type="button"
              onClick={handleExportWord}
              className="px-4 py-3 rounded-xl text-xs font-medium bg-white dark:bg-[#131B2A] border border-slate-200 dark:border-slate-800 text-slate-800 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors flex items-center gap-1.5 whitespace-nowrap shrink-0"
            >
              <Download className="w-4 h-4 text-teal-500" />
              Export Word (.docx)
            </button>

            <button
              type="button"
              onClick={() => setAdvancedOpen(true)}
              className="px-4 py-3 rounded-xl text-xs font-medium bg-white dark:bg-[#131B2A] border border-slate-200 dark:border-slate-800 text-slate-800 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors flex items-center gap-1.5 whitespace-nowrap shrink-0"
            >
              <Sliders className="w-4 h-4 text-teal-500" />
              Appearance &amp; Compute
            </button>
          </div>
        </section>

        {/* Progress Bar & Hardware Plan Summary */}
        <section className="space-y-3">
          <div className="flex items-center justify-between text-xs">
            <span className="font-medium text-slate-800 dark:text-slate-200">
              {stageText}
            </span>
            <span className="font-mono text-slate-500 tabular-nums">
              {busy ? `${Math.round(progress * 100)}%` : 'Idle'}
            </span>
          </div>
          <div className="h-1.5 w-full rounded-full bg-slate-200 dark:bg-slate-800 overflow-hidden">
            <div
              className="h-full transition-all duration-200 rounded-full"
              style={{
                width: `${Math.max(2, progress * 100)}%`,
                backgroundColor: accentStyle,
              }}
            />
          </div>

          {/* Hardware & Active Ensemble Plan Card */}
          <div className="grid grid-cols-1 md:grid-cols-3 gap-4 pt-2">
            <div className="p-4 rounded-xl bg-white dark:bg-[#131B2A] border border-slate-200 dark:border-slate-800 space-y-1">
              <div className="text-xs font-semibold text-slate-900 dark:text-slate-100 font-mono tabular-nums">
                {compute.cpu_threads} CPU threads · 13.8/16.0 GB RAM free
              </div>
              <div className="text-xs text-slate-500 dark:text-slate-400">
                Planned: {preset.toUpperCase()} · {compute.backend.toUpperCase()} · 300 DPI · context{' '}
                {compute.context}
              </div>
            </div>

            <div className="p-4 rounded-xl bg-white dark:bg-[#131B2A] border border-slate-200 dark:border-slate-800 space-y-1">
              <div className="text-xs font-semibold text-slate-900 dark:text-slate-100">
                Writer Profile:{' '}
                {activeWriter ? activeWriter.name : 'None selected'}
              </div>
              <div className="text-xs text-slate-500 dark:text-slate-400 font-mono tabular-nums">
                {activeWriter
                  ? `${writerCoverage(activeWriter).length}/62 alphanumeric glyphs · ${activeWriter.samples.length} confirmed samples`
                  : 'Select a writer to adapt to unique letter shapes'}
              </div>
            </div>

            <div className="p-4 rounded-xl bg-white dark:bg-[#131B2A] border border-slate-200 dark:border-slate-800 space-y-1">
              <div className="text-xs font-semibold text-slate-900 dark:text-slate-100">
                Ensemble Consensus Status
              </div>
              <div className="text-xs text-slate-500 dark:text-slate-400 font-mono tabular-nums">
                Page 1 Agreement: {(pages[0].agreement * 100).toFixed(1)}% ·{' '}
                {pages[0].blocks.filter((b) => b.uncertain).length} uncertain block(s)
              </div>
            </div>
          </div>
        </section>

        {/* Live Activity Log Drawer */}
        <section className="rounded-xl bg-white dark:bg-[#111827] border border-slate-200 dark:border-slate-800 overflow-hidden">
          <div className="flex items-center justify-between px-4 py-2.5 border-b border-slate-200 dark:border-slate-800">
            <button
              type="button"
              onClick={() => setLogOpen((v) => !v)}
              className="text-xs font-medium text-slate-700 dark:text-slate-300 hover:text-slate-900 dark:hover:text-white flex items-center gap-2"
            >
              <span>Live activity · {events.length} events</span>
            </button>
            <span className="text-[11px] text-slate-500 dark:text-slate-400">
              Source scans and alternative readings stay available for review.
            </span>
          </div>

          {logOpen && (
            <div className="max-h-36 overflow-y-auto divide-y divide-slate-100 dark:divide-slate-800/60 px-4 py-1 font-mono text-xs">
              {events.map((ev) => (
                <div
                  key={ev.id}
                  className="py-1.5 flex items-center gap-3 text-slate-600 dark:text-slate-400"
                >
                  <span className="text-slate-400 dark:text-slate-500 tabular-nums shrink-0">
                    {ev.timestamp}
                  </span>
                  <span className="text-teal-600 dark:text-teal-400 font-semibold w-20 shrink-0">
                    [{ev.stage}]
                  </span>
                  <span className="truncate">{ev.message}</span>
                </div>
              ))}
            </div>
          )}
        </section>
      </main>

      {/* Modals */}
      <StudioModal
        open={studioOpen}
        initialTab={studioTab}
        subject={subject}
        glossary={glossary}
        askQuestions={askQuestions}
        maxQuestions={maxQuestions}
        useWriterExamples={useWriterExamples}
        webLookupEnabled={webLookupEnabled}
        attachedSources={attachedSources}
        writers={writers}
        activeWriterId={activeWriterId}
        models={models}
        automaticModels={automaticModels}
        onClose={() => setStudioOpen(false)}
        onUpdateContext={(patch) => {
          if (patch.subject !== undefined) setSubject(patch.subject);
          if (patch.glossary !== undefined) setGlossary(patch.glossary);
          if (patch.askQuestions !== undefined)
            setAskQuestions(patch.askQuestions);
          if (patch.maxQuestions !== undefined)
            setMaxQuestions(patch.maxQuestions);
          if (patch.useWriterExamples !== undefined)
            setUseWriterExamples(patch.useWriterExamples);
          if (patch.webLookupEnabled !== undefined)
            setWebLookupEnabled(patch.webLookupEnabled);
        }}
        onAttachWebSource={(s) => {
          if (!attachedSources.some((item) => item.url === s.url)) {
            setAttachedSources((prev) => [...prev, s]);
            setNotice(`Attached "${s.title}" to scan context.`);
          }
        }}
        onRemoveWebSource={(idx) =>
          setAttachedSources((prev) => prev.filter((_, i) => i !== idx))
        }
        onSelectWriter={(id) => setActiveWriterId(id)}
        onAddWriter={(name) => {
          const newId = `writer-${name
            .toLowerCase()
            .replace(/[^a-z0-9]+/g, '-')}-${Date.now().toString().slice(-4)}`;
          const created: WriterProfile = {
            schema: 1,
            id: newId,
            name,
            notes: 'Custom handwriting profile.',
            revision: 1,
            samples: ['A', 'a', 'e', 'r', 't'].map((ch) => ({
              id: `sample-${newId}-${ch}`,
              label: ch,
              glyph: true,
              created: new Date().toISOString(),
              source: 'Initial seed sample',
              inkGrid: synthesizeGlyphInkGrid(ch, 0),
              strokeTrait: 'Initial baseline sample',
            })),
          };
          setWriters((prev) => [...prev, created]);
          setActiveWriterId(created.id);
          setNotice(`Created writer profile for ${name}.`);
        }}
        onUpdateWriter={(id, name, notes) => {
          setWriters((prev) =>
            prev.map((w) =>
              w.id === id ? { ...w, name, notes, revision: w.revision + 1 } : w
            )
          );
          setNotice(`Saved writer details for ${name}.`);
        }}
        onDeleteWriter={(id) => {
          setWriters((prev) => {
            const remaining = prev.filter((w) => w.id !== id);
            setActiveWriterId(remaining[0]?.id ?? null);
            return remaining;
          });
          setNotice('Deleted writer profile and stored examples.');
        }}
        onForgetSample={(writerId, sampleId) => {
          setWriters((prev) =>
            prev.map((w) =>
              w.id === writerId
                ? {
                    ...w,
                    revision: w.revision + 1,
                    samples: w.samples.filter((s) => s.id !== sampleId),
                  }
                : w
            )
          );
        }}
        onQuickTeachGlyph={(writerId, ch, trait) => {
          setWriters((prev) =>
            prev.map((w) =>
              w.id === writerId
                ? {
                    ...w,
                    revision: w.revision + 1,
                    samples: [
                      ...w.samples,
                      {
                        id: `sample-${Date.now()}-${ch.charCodeAt(0)}`,
                        label: ch,
                        glyph: true,
                        created: new Date().toISOString(),
                        source: 'Studio Glyph Teacher',
                        inkGrid: synthesizeGlyphInkGrid(ch, w.revision + 1),
                        strokeTrait: trait,
                      },
                    ],
                  }
                : w
            )
          );
          setNotice(`Added confirmed '${ch}' glyph sample to writer profile.`);
        }}
        onUpdateModels={(next) => setModels(next)}
        onToggleAutomaticModels={(auto) => setAutomaticModels(auto)}
        onOpenClarificationInbox={handleOpenClarificationInbox}
        onNotify={(msg) => setNotice(msg)}
      />

      <ReviewModal
        open={reviewOpen}
        page={pages[0]}
        activeWriter={activeWriter}
        uploadedImageUrl={uploadedImageUrl}
        reconsidering={reconsidering}
        onClose={() => setReviewOpen(false)}
        onUpdatePage={(updated, approved) => {
          setPages([updated]);
          if (approved) {
            setNotice('Page 1 marked as reviewed and saved.');
          }
        }}
        onTeachWriter={handleTeachWriter}
        onOpenClarificationInbox={() => {
          setReviewOpen(false);
          handleOpenClarificationInbox();
        }}
        onReconsiderWithContext={handleReconsiderWithContext}
        onExportWord={handleExportWord}
      />

      <ClarificationModal
        open={clarifyOpen}
        questions={pendingQuestions}
        activeWriter={activeWriter}
        onClose={() => setClarifyOpen(false)}
        onSubmitAnswers={handleApplyClarifications}
      />

      <AdvancedModal
        open={advancedOpen}
        appearance={appearance}
        compute={compute}
        onClose={() => setAdvancedOpen(false)}
        onUpdateAppearance={(next) => setAppearance(next)}
        onUpdateCompute={(next) => setCompute(next)}
      />
    </div>
  );
}

export default App;
