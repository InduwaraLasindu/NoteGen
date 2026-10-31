import React, { useState } from 'react';
import {
  Block,
  BoxRect,
  PageDocument,
  WriterProfile,
} from '../utils/engine';
import { PageScanCanvas } from './PageScanCanvas';
import {
  Check,
  Crop,
  Download,
  FileCode,
  HelpCircle,
  RefreshCw,
  Save,
  X,
} from 'lucide-react';

interface ReviewModalProps {
  open: boolean;
  page: PageDocument;
  activeWriter: WriterProfile | null;
  uploadedImageUrl?: string | null;
  reconsidering: boolean;
  onClose: () => void;
  onUpdatePage: (updated: PageDocument, markApproved: boolean) => void;
  onTeachWriter: (
    label: string,
    glyph: boolean,
    notes: string,
    region: BoxRect
  ) => void;
  onOpenClarificationInbox: () => void;
  onReconsiderWithContext: () => void;
  onExportWord: () => void;
}

export const ReviewModal: React.FC<ReviewModalProps> = ({
  open,
  page,
  activeWriter,
  uploadedImageUrl,
  reconsidering,
  onClose,
  onUpdatePage,
  onTeachWriter,
  onOpenClarificationInbox,
  onReconsiderWithContext,
  onExportWord,
}) => {
  const [selectedIdx, setSelectedIdx] = useState(0);
  const [cropMode, setCropMode] = useState(false);
  const [customCrop, setCustomCrop] = useState<BoxRect | null>({
    left: 0.195,
    top: 0.23,
    right: 0.355,
    bottom: 0.30,
  });
  const [teachLabel, setTeachLabel] = useState('r');
  const [teachGlyph, setTeachGlyph] = useState(true);
  const [teachNotes, setTeachNotes] = useState(
    'Lowercase r with looped right shoulder'
  );
  const [showRawJson, setShowRawJson] = useState(false);
  const [rawJsonText, setRawJsonText] = useState('');
  const [jsonError, setJsonError] = useState('');

  if (!open) return null;

  const safeIdx = Math.min(selectedIdx, Math.max(0, page.blocks.length - 1));
  const currentBlock: Block | undefined = page.blocks[safeIdx];

  const updateCurrentBlock = (patch: Partial<Block>) => {
    const nextBlocks = page.blocks.map((b, i) =>
      i === safeIdx ? { ...b, ...patch } : b
    );
    onUpdatePage(
      {
        ...page,
        blocks: nextBlocks,
        edited: true,
      },
      false
    );
  };

  const handleToggleRawJson = () => {
    if (!showRawJson) {
      setRawJsonText(JSON.stringify(page.blocks, null, 2));
      setJsonError('');
    }
    setShowRawJson(!showRawJson);
  };

  const handleApplyRawJson = () => {
    try {
      const parsed = JSON.parse(rawJsonText);
      if (!Array.isArray(parsed)) {
        throw new Error('Root must be an array of Block objects.');
      }
      onUpdatePage(
        {
          ...page,
          blocks: parsed,
          edited: true,
        },
        false
      );
      setJsonError('');
      setShowRawJson(false);
    } catch (e) {
      setJsonError(e instanceof Error ? e.message : 'Invalid JSON');
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Review and correct"
      className="fixed inset-0 z-40 flex items-center justify-center bg-black/80 p-3 sm:p-6"
    >
      <div className="w-full max-w-6xl h-[90vh] flex flex-col rounded-xl bg-white dark:bg-[#131B2A] border border-slate-200 dark:border-slate-800 shadow-2xl overflow-hidden">
        {/* Top Header */}
        <div className="flex flex-wrap items-center justify-between gap-4 px-6 py-3.5 border-b border-slate-200 dark:border-slate-800">
          <div className="flex items-center gap-3">
            <h2 className="text-base font-semibold text-slate-900 dark:text-slate-100">
              Review &amp; correct
            </h2>
            <span className="text-xs text-slate-500 dark:text-slate-400 font-mono tabular-nums">
              Page {page.page} / 1 · Agreement {(page.agreement * 100).toFixed(1)}% ·{' '}
              {page.candidates.length} reader families
              {page.reviewed ? ' · Reviewed' : ''}
            </span>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <button
              type="button"
              onClick={onOpenClarificationInbox}
              className="px-3 py-1.5 text-xs font-medium rounded-lg border border-slate-300 dark:border-slate-700 text-slate-700 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors flex items-center gap-1.5 whitespace-nowrap shrink-0"
            >
              <HelpCircle className="w-3.5 h-3.5 text-teal-500" />
              Clarify questions
            </button>
            <button
              type="button"
              disabled={reconsidering}
              onClick={onReconsiderWithContext}
              className="px-3 py-1.5 text-xs font-medium rounded-lg border border-slate-300 dark:border-slate-700 text-slate-700 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 disabled:opacity-50 transition-colors flex items-center gap-1.5 whitespace-nowrap shrink-0"
            >
              <RefreshCw
                className={`w-3.5 h-3.5 text-teal-500 ${
                  reconsidering ? 'animate-spin' : ''
                }`}
              />
              {reconsidering ? 'Re-reading...' : 'Re-read with context'}
            </button>
            <button
              type="button"
              onClick={() => setCropMode((v) => !v)}
              className={`px-3 py-1.5 text-xs font-medium rounded-lg border transition-colors flex items-center gap-1.5 whitespace-nowrap shrink-0 ${
                cropMode
                  ? 'bg-teal-500 text-slate-950 border-teal-500 font-semibold'
                  : 'border-slate-300 dark:border-slate-700 text-slate-700 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800'
              }`}
            >
              <Crop className="w-3.5 h-3.5" />
              {cropMode ? 'Exit crop mode' : 'Label handwriting / context'}
            </button>
            <button
              type="button"
              onClick={onClose}
              aria-label="Close review modal"
              className="p-1.5 rounded-lg text-slate-500 hover:text-slate-900 dark:hover:text-white transition-colors"
            >
              <X className="w-4 h-4" />
            </button>
          </div>
        </div>

        {/* Main 2-Column Split */}
        <div className="flex-1 grid grid-cols-1 lg:grid-cols-12 overflow-hidden">
          {/* Left Column: Interactive Scan Surface */}
          <div className="lg:col-span-6 p-5 overflow-y-auto border-b lg:border-b-0 lg:border-r border-slate-200 dark:border-slate-800 bg-slate-100/70 dark:bg-slate-950/50">
            <PageScanCanvas
              page={page}
              selectedBlockIndex={safeIdx}
              onSelectBlock={(i) => setSelectedIdx(i)}
              cropMode={cropMode}
              customCrop={customCrop}
              onCustomCropChange={(r) => setCustomCrop(r)}
              uploadedImageUrl={uploadedImageUrl}
            />
          </div>

          {/* Right Column: Warnings, Teach Panel (if Crop Mode), Block Editor, Evidence */}
          <div className="lg:col-span-6 p-6 overflow-y-auto space-y-5">
            {/* Warnings List */}
            {page.warnings.length > 0 && (
              <div className="space-y-1.5 p-3.5 rounded-lg bg-amber-500/10 border border-amber-500/30">
                {page.warnings.map((w, i) => (
                  <p
                    key={i}
                    className="text-xs text-amber-800 dark:text-amber-300 leading-relaxed"
                  >
                    {w}
                  </p>
                ))}
              </div>
            )}

            {/* Teach Handwriting / Region Context Panel when Crop Mode is active */}
            {cropMode && customCrop && (
              <div className="p-4 rounded-xl bg-teal-500/10 border border-teal-500/30 space-y-3">
                <div className="flex items-center justify-between">
                  <h3 className="text-xs font-semibold text-teal-800 dark:text-teal-300">
                    Label handwriting / context (Crop{' '}
                    <span className="font-mono tabular-nums">
                      {(customCrop.left * 100).toFixed(0)}%,
                      {(customCrop.top * 100).toFixed(0)}% →{' '}
                      {(customCrop.right * 100).toFixed(0)}%,
                      {(customCrop.bottom * 100).toFixed(0)}%
                    </span>
                    )
                  </h3>
                  <span className="text-[11px] text-slate-500 dark:text-slate-400">
                    Drag on scan to adjust rectangle
                  </span>
                </div>

                <div>
                  <label
                    htmlFor="teach-label-input"
                    className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1"
                  >
                    What does this area say or represent?
                  </label>
                  <input
                    id="teach-label-input"
                    type="text"
                    value={teachLabel}
                    onChange={(e) => setTeachLabel(e.target.value)}
                    placeholder="Exact confirmed letter (e.g. r), word, or number"
                    className="w-full rounded-lg bg-white dark:bg-slate-900 border border-slate-300 dark:border-slate-700 px-3 py-1.5 text-sm text-slate-900 dark:text-slate-100"
                  />
                </div>

                <label className="flex items-center gap-2 text-xs text-slate-700 dark:text-slate-300 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={teachGlyph}
                    onChange={(e) => setTeachGlyph(e.target.checked)}
                    className="rounded border-slate-400 text-teal-500"
                  />
                  <span>
                    This is one isolated English letter / digit / punctuation mark (builds personal TrueType font)
                  </span>
                </label>

                <div>
                  <label
                    htmlFor="teach-notes-input"
                    className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1"
                  >
                    Optional stroke trait, meaning, or context note
                  </label>
                  <input
                    id="teach-notes-input"
                    type="text"
                    value={teachNotes}
                    onChange={(e) => setTeachNotes(e.target.value)}
                    placeholder="e.g. Looped right shoulder on lowercase r"
                    className="w-full rounded-lg bg-white dark:bg-slate-900 border border-slate-300 dark:border-slate-700 px-3 py-1.5 text-xs text-slate-900 dark:text-slate-100"
                  />
                </div>

                <div className="flex items-center gap-2 pt-1">
                  <button
                    type="button"
                    disabled={!teachLabel.trim()}
                    onClick={() => {
                      onTeachWriter(
                        teachLabel.trim(),
                        teachGlyph,
                        teachNotes.trim(),
                        customCrop
                      );
                    }}
                    className="px-3.5 py-1.5 text-xs font-semibold rounded-lg bg-teal-500 text-slate-950 hover:bg-teal-400 disabled:opacity-40 transition-colors"
                  >
                    Teach {activeWriter ? activeWriter.name : 'active writer'}
                  </button>
                </div>
              </div>
            )}

            {/* Block Selector Tabs */}
            <div className="flex flex-wrap items-center gap-1.5 pb-2 border-b border-slate-200 dark:border-slate-800">
              {page.blocks.map((b, i) => (
                <button
                  key={b.id}
                  type="button"
                  onClick={() => setSelectedIdx(i)}
                  className={`px-3 py-1.5 text-xs font-medium rounded-lg transition-colors whitespace-nowrap shrink-0 ${
                    safeIdx === i
                      ? 'bg-teal-500/20 text-teal-700 dark:text-teal-300 border border-teal-500/40'
                      : 'bg-slate-100 dark:bg-slate-800/70 text-slate-600 dark:text-slate-400 hover:text-slate-900 dark:hover:text-white'
                  }`}
                >
                  {i + 1} {b.kind}
                  {b.uncertain ? ' · uncertain' : ''}
                </button>
              ))}
            </div>

            {/* Active Block Editor */}
            {currentBlock && (
              <div className="space-y-4">
                <div className="flex items-center justify-between">
                  <span className="text-xs font-mono text-slate-500 dark:text-slate-400">
                    Block #{safeIdx + 1} ({currentBlock.kind}) · Confidence{' '}
                    {(currentBlock.confidence * 100).toFixed(0)}%
                  </span>
                  <label className="flex items-center gap-2 text-xs text-slate-700 dark:text-slate-300 cursor-pointer">
                    <input
                      type="checkbox"
                      checked={currentBlock.uncertain}
                      onChange={(e) =>
                        updateCurrentBlock({ uncertain: e.target.checked })
                      }
                      className="rounded border-slate-400 text-teal-500"
                    />
                    <span>Uncertain (requires review)</span>
                  </label>
                </div>

                {currentBlock.kind === 'table' ? (
                  <div className="space-y-2">
                    <p className="text-xs font-medium text-slate-700 dark:text-slate-300">
                      Editable Table Cells (exported as native Word TableGrid cells)
                    </p>
                    <div className="border border-slate-300 dark:border-slate-700 rounded-lg overflow-hidden">
                      {currentBlock.rows.map((row, rIdx) => (
                        <div
                          key={rIdx}
                          className={`grid grid-cols-3 border-b last:border-b-0 border-slate-200 dark:border-slate-800 ${
                            rIdx < currentBlock.header_rows
                              ? 'bg-slate-100 dark:bg-slate-800/80 font-semibold'
                              : 'bg-white dark:bg-slate-900'
                          }`}
                        >
                          {row.map((cell, cIdx) => (
                            <input
                              key={cIdx}
                              type="text"
                              aria-label={`Table row ${rIdx + 1} column ${cIdx + 1}`}
                              value={cell}
                              onChange={(e) => {
                                const nextRows = currentBlock.rows.map((r, ri) =>
                                  ri === rIdx
                                    ? r.map((c, ci) =>
                                        ci === cIdx ? e.target.value : c
                                      )
                                    : [...r]
                                );
                                updateCurrentBlock({
                                  rows: nextRows,
                                  uncertain: false,
                                });
                              }}
                              className="px-2.5 py-2 text-xs font-mono text-slate-900 dark:text-slate-100 border-r last:border-r-0 border-slate-200 dark:border-slate-800 bg-transparent focus:outline-none focus:bg-teal-500/10"
                            />
                          ))}
                        </div>
                      ))}
                    </div>
                  </div>
                ) : currentBlock.kind === 'figure' ? (
                  <div className="space-y-3">
                    <div>
                      <label
                        htmlFor="figure-observed-input"
                        className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1"
                      >
                        Visible evidence (Observed in diagram crop)
                      </label>
                      <textarea
                        id="figure-observed-input"
                        rows={2}
                        value={currentBlock.description}
                        onChange={(e) =>
                          updateCurrentBlock({ description: e.target.value })
                        }
                        className="w-full rounded-lg bg-white dark:bg-slate-950 border border-slate-300 dark:border-slate-700 px-3 py-2 text-xs text-slate-900 dark:text-slate-100"
                      />
                    </div>
                    <div>
                      <label
                        htmlFor="figure-interpretation-input"
                        className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1"
                      >
                        Contextual interpretation (Preserved separately in Word export)
                      </label>
                      <textarea
                        id="figure-interpretation-input"
                        rows={3}
                        value={currentBlock.interpretation}
                        onChange={(e) =>
                          updateCurrentBlock({
                            interpretation: e.target.value,
                            uncertain: false,
                          })
                        }
                        className="w-full rounded-lg bg-white dark:bg-slate-950 border border-slate-300 dark:border-slate-700 px-3 py-2 text-xs text-slate-900 dark:text-slate-100"
                      />
                    </div>
                  </div>
                ) : (
                  <div>
                    <label
                      htmlFor="block-text-editor"
                      className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1"
                    >
                      Recognized transcript ({currentBlock.kind})
                    </label>
                    <textarea
                      id="block-text-editor"
                      rows={4}
                      value={currentBlock.text}
                      onChange={(e) =>
                        updateCurrentBlock({
                          text: e.target.value,
                          uncertain: e.target.value.includes('[illegible]'),
                        })
                      }
                      className="w-full rounded-lg bg-white dark:bg-slate-950 border border-slate-300 dark:border-slate-700 px-3 py-2.5 text-sm text-slate-900 dark:text-slate-100 focus:outline-none focus:ring-2 focus:ring-teal-500"
                    />
                  </div>
                )}

                {/* Alternative Reader Evidence */}
                <div className="p-3.5 rounded-lg bg-slate-50 dark:bg-slate-900/60 border border-slate-200 dark:border-slate-800 space-y-1.5">
                  <div className="text-xs font-semibold text-slate-700 dark:text-slate-300">
                    All reader evidence ({currentBlock.alternatives.length} readings)
                  </div>
                  {currentBlock.alternatives.map((alt, i) => (
                    <div
                      key={i}
                      className="text-xs font-mono text-slate-600 dark:text-slate-400"
                    >
                      {alt}
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Raw Page JSON Inspector */}
            <div className="pt-2 border-t border-slate-200 dark:border-slate-800">
              <button
                type="button"
                onClick={handleToggleRawJson}
                className="text-xs font-medium text-slate-600 dark:text-slate-400 hover:text-slate-900 dark:hover:text-white flex items-center gap-1.5"
              >
                <FileCode className="w-3.5 h-3.5" />
                {showRawJson ? 'Hide page JSON' : 'Edit page JSON'}
              </button>

              {showRawJson && (
                <div className="mt-2 space-y-2">
                  <textarea
                    rows={7}
                    aria-label="Raw page blocks JSON"
                    value={rawJsonText}
                    onChange={(e) => setRawJsonText(e.target.value)}
                    className="w-full rounded-lg bg-slate-950 text-slate-200 font-mono text-xs p-3 border border-slate-800"
                  />
                  {jsonError && (
                    <p className="text-xs text-red-400">{jsonError}</p>
                  )}
                  <button
                    type="button"
                    onClick={handleApplyRawJson}
                    className="px-3 py-1.5 text-xs font-semibold rounded-lg bg-teal-500 text-slate-950"
                  >
                    Validate &amp; apply JSON
                  </button>
                </div>
              )}
            </div>
          </div>
        </div>

        {/* Bottom Action Bar */}
        <div className="flex flex-wrap items-center justify-between gap-3 px-6 py-3.5 bg-slate-50 dark:bg-slate-900/70 border-t border-slate-200 dark:border-slate-800">
          <div className="flex flex-wrap items-center gap-2">
            <button
              type="button"
              onClick={() => onUpdatePage({ ...page, edited: true }, false)}
              className="px-3.5 py-2 text-xs font-medium rounded-lg border border-slate-300 dark:border-slate-700 text-slate-700 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors flex items-center gap-1.5"
            >
              <Save className="w-3.5 h-3.5" />
              Save draft
            </button>
            <button
              type="button"
              onClick={() =>
                onUpdatePage(
                  {
                    ...page,
                    edited: true,
                    reviewed: true,
                    blocks: page.blocks.map((b) => ({ ...b, uncertain: false })),
                  },
                  true
                )
              }
              className="px-3.5 py-2 text-xs font-semibold rounded-lg bg-teal-500 text-slate-950 hover:bg-teal-400 transition-colors flex items-center gap-1.5"
            >
              <Check className="w-3.5 h-3.5" />
              Save &amp; mark reviewed
            </button>
            <button
              type="button"
              onClick={onExportWord}
              className="px-3.5 py-2 text-xs font-medium rounded-lg border border-slate-300 dark:border-slate-700 text-slate-700 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors flex items-center gap-1.5"
            >
              <Download className="w-3.5 h-3.5" />
              Export corrected Word (.docx)
            </button>
          </div>

          <span className="text-[11px] text-slate-500 dark:text-slate-400">
            Drafts preserve your edits across resume. Mark reviewed only after comparing with the source.
          </span>
        </div>
      </div>
    </div>
  );
};
