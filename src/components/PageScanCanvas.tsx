import React, { useRef, useState } from 'react';
import { Block, BoxRect, PageDocument } from '../utils/engine';

interface PageScanCanvasProps {
  page: PageDocument;
  selectedBlockIndex: number;
  onSelectBlock: (index: number) => void;
  cropMode: boolean;
  customCrop: BoxRect | null;
  onCustomCropChange: (rect: BoxRect) => void;
  uploadedImageUrl?: string | null;
}

export const PageScanCanvas: React.FC<PageScanCanvasProps> = ({
  page,
  selectedBlockIndex,
  onSelectBlock,
  cropMode,
  customCrop,
  onCustomCropChange,
  uploadedImageUrl,
}) => {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [dragStart, setDragStart] = useState<{ x: number; y: number } | null>(
    null
  );

  const getNormalizedCoords = (e: React.MouseEvent) => {
    const rect = containerRef.current?.getBoundingClientRect();
    if (!rect) return { x: 0.1, y: 0.1 };
    const x = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
    const y = Math.min(1, Math.max(0, (e.clientY - rect.top) / rect.height));
    return { x, y };
  };

  const handleMouseDown = (e: React.MouseEvent) => {
    if (!cropMode) return;
    const pt = getNormalizedCoords(e);
    setDragStart(pt);
    onCustomCropChange({
      left: pt.x,
      top: pt.y,
      right: Math.min(1, pt.x + 0.08),
      bottom: Math.min(1, pt.y + 0.06),
    });
  };

  const handleMouseMove = (e: React.MouseEvent) => {
    if (!cropMode || !dragStart) return;
    const pt = getNormalizedCoords(e);
    const left = Math.min(dragStart.x, pt.x);
    const right = Math.max(dragStart.x, pt.x);
    const top = Math.min(dragStart.y, pt.y);
    const bottom = Math.max(dragStart.y, pt.y);
    if (right - left > 0.02 && bottom - top > 0.02) {
      onCustomCropChange({ left, top, right, bottom });
    }
  };

  const handleMouseUp = () => {
    setDragStart(null);
  };

  return (
    <div
      ref={containerRef}
      onMouseDown={handleMouseDown}
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      data-testid="page-scan-surface"
      className={`relative w-full aspect-[3/4] bg-[#FAF8F2] text-slate-900 rounded-lg border border-slate-300 dark:border-slate-700 overflow-hidden shadow-sm select-none ${
        cropMode ? 'cursor-crosshair' : 'cursor-default'
      }`}
    >
      {/* Subtle engineering notebook grid paper lines */}
      <div
        className="absolute inset-0 pointer-events-none opacity-35"
        style={{
          backgroundImage:
            'linear-gradient(to right, #cbd5e1 1px, transparent 1px), linear-gradient(to bottom, #cbd5e1 1px, transparent 1px)',
          backgroundSize: '24px 24px',
        }}
      />

      {uploadedImageUrl ? (
        <img
          src={uploadedImageUrl}
          alt={page.title}
          referrerPolicy="no-referrer"
          className="absolute inset-0 w-full h-full object-contain"
        />
      ) : (
        /* Authentic rendered facsimile of the Fluid Mechanics handwritten + table + siphon scan */
        <div className="relative z-10 p-6 sm:p-8 flex flex-col justify-between h-full">
          {/* Block 1: Heading */}
          <div className="mt-1">
            <p
              className="text-lg sm:text-xl font-bold text-slate-900 tracking-tight"
              style={{ fontFamily: "'Caveat', cursive", fontSize: '1.65rem' }}
            >
              Section 4.2: Siphon &amp; Venturi Flow Balance
            </p>
          </div>

          {/* Block 2: Handwritten paragraph with distinctive looped 'r' */}
          <div className="my-2 px-2 py-1.5 rounded bg-amber-50/60 border border-dashed border-amber-300/70">
            <p
              className="text-slate-800 leading-snug"
              style={{ fontFamily: "'Caveat', cursive", fontSize: '1.35rem' }}
            >
              In steady incompressible flow between Tank A and Tank B, onset of{' '}
              <span className="underline decoration-amber-600 decoration-wavy font-bold text-slate-950 px-1 bg-amber-200/50 rounded">
                tu<span className="text-teal-800">r</span>bulence
              </span>{' '}
              occurs when the local Reynolds number exceeds{' '}
              <span className="font-mono text-base font-semibold">2300</span>{' '}
              along the constricted throat.
            </p>
          </div>

          {/* Block 3: Ruled experimental table */}
          <div className="my-2 bg-white/90 border-2 border-slate-800 text-xs sm:text-sm font-mono">
            <div className="grid grid-cols-3 border-b-2 border-slate-800 bg-slate-100 font-semibold">
              <div className="p-2 border-r border-slate-800">Conduit Segment</div>
              <div className="p-2 border-r border-slate-800">Flow Q (mL/s)</div>
              <div className="p-2">Static Head h</div>
            </div>
            <div className="grid grid-cols-3 border-b border-slate-800">
              <div className="p-2 border-r border-slate-800">Inlet Reservoir A</div>
              <div className="p-2 border-r border-slate-800 tabular-nums">1250</div>
              <div className="p-2 tabular-nums">48.5 cm</div>
            </div>
            <div className="grid grid-cols-3">
              <div className="p-2 border-r border-slate-800">Venturi Throat B</div>
              <div
                className="p-2 border-r border-slate-800 font-bold text-amber-900 bg-amber-100/70 tabular-nums"
                style={{ fontFamily: "'Caveat', cursive", fontSize: '1.15rem' }}
              >
                1250 mL/s
              </div>
              <div className="p-2 tabular-nums">19.2 cm</div>
            </div>
          </div>

          {/* Block 4: Hand-drawn Siphon & Venturi Diagram */}
          <div className="mt-2 p-3 bg-white/85 border border-slate-400 rounded">
            <svg
              viewBox="0 0 420 140"
              className="w-full h-28 sm:h-32 stroke-slate-800"
              fill="none"
            >
              {/* Tank A */}
              <rect
                x="20"
                y="35"
                width="75"
                height="85"
                strokeWidth="2.2"
                fill="rgba(56, 189, 248, 0.14)"
              />
              <line
                x1="20"
                y1="52"
                x2="95"
                y2="52"
                stroke="#0284c7"
                strokeWidth="2"
                strokeDasharray="4 2"
              />
              <text
                x="35"
                y="85"
                fill="#0f172a"
                fontSize="13"
                fontWeight="bold"
              >
                Tank A
              </text>

              {/* Siphon Crest C & Venturi Throat B */}
              <path
                d="M 65 75 C 65 12, 195 12, 225 58 L 265 78 L 325 98"
                stroke="#0f172a"
                strokeWidth="6"
                strokeLinecap="round"
              />
              <path
                d="M 65 75 C 65 12, 195 12, 225 58 L 265 78 L 325 98"
                stroke="#38bdf8"
                strokeWidth="2.5"
                strokeLinecap="round"
              />
              <circle cx="242" cy="67" r="7" stroke="#d97706" strokeWidth="2" />
              <text
                x="130"
                y="18"
                fill="#0f172a"
                fontSize="11"
                fontWeight="bold"
              >
                Crest C (Q = 1250)
              </text>
              <text
                x="212"
                y="48"
                fill="#b45309"
                fontSize="11"
                fontWeight="bold"
              >
                Throat B (Δh = 29.3 cm)
              </text>

              {/* Tank B */}
              <rect
                x="300"
                y="62"
                width="85"
                height="62"
                strokeWidth="2.2"
                fill="rgba(56, 189, 248, 0.14)"
              />
              <text
                x="318"
                y="105"
                fill="#0f172a"
                fontSize="13"
                fontWeight="bold"
              >
                Tank B
              </text>
            </svg>
          </div>
        </div>
      )}

      {/* Interactive Block Bounding Boxes */}
      {page.blocks.map((block: Block, idx: number) => {
        if (!block.bbox) return null;
        const isSelected = idx === selectedBlockIndex;
        const borderColor = isSelected
          ? 'border-teal-400 bg-teal-400/15 ring-2 ring-teal-400/50'
          : block.uncertain
          ? 'border-amber-500/80 bg-amber-500/10 hover:bg-amber-500/20'
          : 'border-slate-500/50 hover:border-teal-500/70 bg-transparent';

        return (
          <button
            key={block.id}
            type="button"
            aria-label={`Inspect block ${idx + 1} (${block.kind})`}
            onClick={(e) => {
              if (cropMode) return;
              e.stopPropagation();
              onSelectBlock(idx);
            }}
            style={{
              left: `${block.bbox.left * 100}%`,
              top: `${block.bbox.top * 100}%`,
              width: `${(block.bbox.right - block.bbox.left) * 100}%`,
              height: `${(block.bbox.bottom - block.bbox.top) * 100}%`,
            }}
            className={`absolute z-20 border-2 rounded transition-colors text-left ${borderColor}`}
          >
            <span className="inline-block px-1.5 py-0.5 text-[10px] font-mono font-semibold bg-slate-900/85 text-white rounded-br">
              #{idx + 1} {block.kind}
              {block.uncertain ? ' · review' : ''}
            </span>
          </button>
        );
      })}

      {/* Active Custom Crop Selection Box in Crop Mode */}
      {cropMode && customCrop && (
        <div
          style={{
            left: `${customCrop.left * 100}%`,
            top: `${customCrop.top * 100}%`,
            width: `${(customCrop.right - customCrop.left) * 100}%`,
            height: `${(customCrop.bottom - customCrop.top) * 100}%`,
          }}
          className="absolute z-30 border-2 border-teal-400 bg-teal-400/25 pointer-events-none rounded shadow-sm"
        >
          <span className="px-1.5 py-0.5 text-[10px] font-mono font-semibold bg-teal-500 text-slate-950">
            Selected Crop
          </span>
        </div>
      )}
    </div>
  );
};
