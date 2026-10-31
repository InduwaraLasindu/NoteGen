import React, { useState } from 'react';
import {
  ClarificationAnswer,
  ClarificationQuestion,
  WriterProfile,
} from '../utils/engine';
import { Check, ChevronLeft, ChevronRight, HelpCircle, X } from 'lucide-react';

interface ClarificationModalProps {
  open: boolean;
  questions: ClarificationQuestion[];
  activeWriter: WriterProfile | null;
  onClose: () => void;
  onSubmitAnswers: (answers: ClarificationAnswer[]) => void;
}

export const ClarificationModal: React.FC<ClarificationModalProps> = ({
  open,
  questions,
  activeWriter,
  onClose,
  onSubmitAnswers,
}) => {
  const [index, setIndex] = useState(0);
  const [answers, setAnswers] = useState<ClarificationAnswer[]>(() =>
    questions.map((q) => ({
      id: q.id,
      replacement: '',
      context: '',
      skip: false,
      learn: Boolean(activeWriter && q.field === 'text'),
      glyph: false,
    }))
  );
  const [error, setError] = useState('');

  React.useEffect(() => {
    setIndex(0);
    setAnswers(
      questions.map((q) => ({
        id: q.id,
        replacement:
          q.field === 'text' && q.reading.includes('[illegible]')
            ? q.reading.replace('[illegible]', 'turbulence')
            : q.field.startsWith('cell:') && q.reading.includes('[illegible]')
            ? '1250'
            : '',
        context: '',
        skip: false,
        learn: Boolean(activeWriter && q.field === 'text'),
        glyph: false,
      }))
    );
    setError('');
  }, [questions, activeWriter]);

  if (!open || questions.length === 0) return null;

  const safeIdx = Math.min(index, questions.length - 1);
  const currentQ = questions[safeIdx];
  const currentA = answers[safeIdx] ?? {
    id: currentQ.id,
    replacement: '',
    context: '',
    skip: false,
    learn: false,
    glyph: false,
  };

  const updateCurrentAnswer = (patch: Partial<ClarificationAnswer>) => {
    setAnswers((prev) =>
      prev.map((item, idx) => (idx === safeIdx ? { ...item, ...patch } : item))
    );
    setError('');
  };

  const handleApplyAll = () => {
    for (let i = 0; i < answers.length; i++) {
      const a = answers[i];
      if (!a.skip && a.learn && a.glyph && a.replacement.trim().length !== 1) {
        setError(
          `NG630 (Question ${i + 1}): When "One isolated English glyph" is checked, the replacement must be a single character (e.g. "r"). Uncheck isolated glyph to save a full word/phrase.`
        );
        return;
      }
    }
    onSubmitAnswers(answers);
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="A little help with this page"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-4"
    >
      <div className="w-full max-w-2xl rounded-xl bg-white dark:bg-[#131B2A] border border-slate-200 dark:border-slate-800 shadow-xl overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-slate-200 dark:border-slate-800">
          <div className="flex items-center gap-2.5">
            <HelpCircle className="w-5 h-5 text-teal-500 shrink-0" />
            <h2 className="text-base font-semibold text-slate-900 dark:text-slate-100">
              A little help with this page
            </h2>
            <span className="text-xs text-slate-500 dark:text-slate-400 font-mono tabular-nums">
              · Question {safeIdx + 1} of {questions.length} · Page {currentQ.page}
            </span>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close clarification dialog"
            className="p-1.5 rounded-lg text-slate-500 hover:text-slate-900 dark:hover:text-white transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Body */}
        <div className="p-6 space-y-4 max-h-[78vh] overflow-y-auto">
          <p className="text-sm font-semibold text-slate-900 dark:text-slate-100">
            {currentQ.prompt}
          </p>

          {/* Visual Simulated Source Crop Preview */}
          <div className="p-4 rounded-lg bg-[#FAF8F2] border border-slate-300 text-slate-900">
            <div className="text-[11px] font-mono text-slate-500 mb-1.5 flex items-center justify-between">
              <span>
                Source Crop · Block #{currentQ.blockIndex + 1} ({currentQ.field})
              </span>
              {activeWriter && (
                <span>Writer profile: {activeWriter.name}</span>
              )}
            </div>
            {currentQ.field === 'meaning' ? (
              <div className="text-xs font-mono text-slate-800 py-2">
                [Diagram Crop: Two-tank Siphon &amp; Venturi Conduit — Crest C (Q=1250), Throat B (Δh=29.3 cm)]
              </div>
            ) : (
              <p
                className="text-slate-900 leading-snug"
                style={{ fontFamily: "'Caveat', cursive", fontSize: '1.55rem' }}
              >
                {currentQ.reading}
              </p>
            )}
          </div>

          {/* Current Reading & Candidate Alternatives */}
          <div className="p-3.5 rounded-lg bg-slate-50 dark:bg-slate-900/70 border border-slate-200 dark:border-slate-800 space-y-1.5">
            <div className="text-xs text-slate-700 dark:text-slate-300">
              <span className="font-semibold">Current reading: </span>
              <span className="font-mono">{currentQ.reading}</span>
            </div>
            {currentQ.alternatives.map((alt, i) => (
              <div
                key={i}
                className="text-xs text-slate-500 dark:text-slate-400 font-mono flex items-center justify-between gap-2"
              >
                <span className="truncate">Alternative: {alt}</span>
                <button
                  type="button"
                  onClick={() => {
                    const colon = alt.indexOf(':');
                    const text = colon >= 0 ? alt.slice(colon + 1).trim() : alt;
                    updateCurrentAnswer({ replacement: text, skip: false });
                  }}
                  className="text-teal-600 dark:text-teal-400 hover:underline shrink-0 text-[11px]"
                >
                  Use reading
                </button>
              </div>
            ))}
          </div>

          {/* Replacement Input */}
          <div>
            <label
              htmlFor="clarify-replacement-input"
              className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1.5"
            >
              {currentQ.field === 'meaning'
                ? 'Your explanation of this diagram (optional)'
                : 'Exact replacement for the text / table cell shown above (optional)'}
            </label>
            <textarea
              id="clarify-replacement-input"
              rows={3}
              value={currentA.replacement}
              onChange={(e) =>
                updateCurrentAnswer({
                  replacement: e.target.value,
                  skip: false,
                })
              }
              placeholder="Enter exact confirmed text, number, or diagram interpretation..."
              className="w-full rounded-lg bg-white dark:bg-slate-950 border border-slate-300 dark:border-slate-700 px-3 py-2 text-sm text-slate-900 dark:text-slate-100 focus:outline-none focus:ring-2 focus:ring-teal-500"
            />
          </div>

          {/* Extra Context Input */}
          <div>
            <label
              htmlFor="clarify-context-input"
              className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1.5"
            >
              Extra context for a new AI reading (optional)
            </label>
            <input
              id="clarify-context-input"
              type="text"
              value={currentA.context}
              onChange={(e) =>
                updateCurrentAnswer({ context: e.target.value, skip: false })
              }
              placeholder="e.g., Lowercase 'r' in turbulence has a high right loop; Q is in mL/s..."
              className="w-full rounded-lg bg-white dark:bg-slate-950 border border-slate-300 dark:border-slate-700 px-3 py-2 text-sm text-slate-900 dark:text-slate-100 focus:outline-none focus:ring-2 focus:ring-teal-500"
            />
          </div>

          {/* Writer Learning Checkboxes */}
          {activeWriter && currentQ.field === 'text' && (
            <div className="pt-1 space-y-2">
              <label className="flex items-center gap-2.5 text-xs text-slate-700 dark:text-slate-300 cursor-pointer">
                <input
                  type="checkbox"
                  checked={currentA.learn}
                  onChange={(e) =>
                    updateCurrentAnswer({ learn: e.target.checked })
                  }
                  className="rounded border-slate-400 text-teal-500 focus:ring-teal-500"
                />
                <span>
                  Remember this confirmed crop for writer{' '}
                  <strong className="text-slate-900 dark:text-white">
                    {activeWriter.name}
                  </strong>
                </span>
              </label>

              {currentA.learn && (
                <label className="flex items-center gap-2.5 pl-6 text-xs text-slate-600 dark:text-slate-400 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={currentA.glyph}
                    onChange={(e) =>
                      updateCurrentAnswer({ glyph: e.target.checked })
                    }
                    className="rounded border-slate-400 text-teal-500 focus:ring-teal-500"
                  />
                  <span>
                    One isolated English glyph (single letter/digit — builds personal TrueType font)
                  </span>
                </label>
              )}
            </div>
          )}

          {error && (
            <div className="p-3 rounded-lg bg-red-500/10 border border-red-500/30 text-xs text-red-600 dark:text-red-300">
              {error}
            </div>
          )}

          <p className="text-[11px] text-slate-500 dark:text-slate-400">
            Leave the correction empty when unsure. Context can request a new AI vision reading; it never silently replaces confirmed text.
          </p>
        </div>

        {/* Footer Controls */}
        <div className="flex items-center justify-between gap-3 px-6 py-4 bg-slate-50 dark:bg-slate-900/50 border-t border-slate-200 dark:border-slate-800">
          <div className="flex items-center gap-2">
            <button
              type="button"
              disabled={safeIdx === 0}
              onClick={() => setIndex((i) => Math.max(0, i - 1))}
              className="px-3 py-1.5 text-xs font-medium rounded-lg border border-slate-300 dark:border-slate-700 text-slate-700 dark:text-slate-300 disabled:opacity-40 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors flex items-center gap-1 whitespace-nowrap shrink-0"
            >
              <ChevronLeft className="w-3.5 h-3.5" />
              Previous
            </button>
            <button
              type="button"
              onClick={() => {
                updateCurrentAnswer({ skip: true });
                if (safeIdx + 1 < questions.length) {
                  setIndex(safeIdx + 1);
                } else {
                  handleApplyAll();
                }
              }}
              className="px-3 py-1.5 text-xs font-medium rounded-lg border border-slate-300 dark:border-slate-700 text-slate-700 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors whitespace-nowrap shrink-0"
            >
              Skip this
            </button>
            <button
              type="button"
              disabled={safeIdx + 1 >= questions.length}
              onClick={() => setIndex((i) => Math.min(questions.length - 1, i + 1))}
              className="px-3 py-1.5 text-xs font-medium rounded-lg border border-slate-300 dark:border-slate-700 text-slate-700 dark:text-slate-300 disabled:opacity-40 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors flex items-center gap-1 whitespace-nowrap shrink-0"
            >
              Next
              <ChevronRight className="w-3.5 h-3.5" />
            </button>
          </div>

          <button
            type="button"
            onClick={handleApplyAll}
            className="px-4 py-2 text-xs font-semibold rounded-lg bg-teal-500 text-slate-950 hover:bg-teal-400 transition-colors flex items-center gap-1.5 whitespace-nowrap shrink-0"
          >
            <Check className="w-3.5 h-3.5" />
            Apply answers &amp; continue
          </button>
        </div>
      </div>
    </div>
  );
};
