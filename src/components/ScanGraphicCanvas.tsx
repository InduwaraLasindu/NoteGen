import React, { useEffect, useRef } from 'react';
import { AppearanceSettings } from '../utils/engine';

interface ScanGraphicCanvasProps {
  appearance: AppearanceSettings;
  busy: boolean;
  progress: number;
  completed: boolean;
}

export const ScanGraphicCanvas: React.FC<ScanGraphicCanvasProps> = ({
  appearance,
  busy,
  progress,
  completed,
}) => {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    let animId = 0;
    const start = performance.now();

    const draw = (now: number) => {
      const elapsed = (now - start) / 1000;
      const [r, g, b] = appearance.accent;
      const motionEnabled =
        !appearance.reduced_motion &&
        appearance.motion > 0 &&
        (appearance.scan_animation || appearance.particles);

      const speed = Math.min(3, Math.max(0.1, appearance.motion));
      const period = busy ? 2.2 : 4.6;
      const phase = motionEnabled
        ? ((elapsed * speed) % period) / period
        : 0.5;
      const scanY = 0.5 - 0.5 * Math.cos(phase * Math.PI * 2);

      const w = canvas.width;
      const h = canvas.height;
      ctx.clearRect(0, 0, w, h);

      const cx = w / 2;
      const cy = h / 2;

      // Subtle concentric radar / orbital rings (matches docs/images/rust-workspace.png)
      for (const radius of [58, 76, 94]) {
        ctx.beginPath();
        ctx.arc(cx, cy, radius, 0, Math.PI * 2);
        ctx.strokeStyle = `rgba(${r}, ${g}, ${b}, 0.08)`;
        ctx.lineWidth = 1;
        ctx.stroke();
      }

      // Orbiting particles controlled by ambient_density
      if (appearance.particles) {
        const count = Math.min(6, Math.max(0, appearance.ambient_density));
        for (let i = 0; i < count; i++) {
          const angle =
            (motionEnabled ? elapsed * speed * 0.65 : 0.4) +
            (i * Math.PI * 2) / Math.max(1, count);
          const orbitRadius = 64 + (i % 2) * 18;
          const px = cx + Math.cos(angle) * orbitRadius;
          const py = cy + Math.sin(angle) * orbitRadius;
          ctx.beginPath();
          ctx.arc(px, py, 2.5, 0, Math.PI * 2);
          ctx.fillStyle = `rgba(${r}, ${g}, ${b}, 0.65)`;
          ctx.fill();
        }
      }

      // Document page frame
      const docW = 76;
      const docH = 98;
      const docX = cx - docW / 2;
      const docY = cy - docH / 2;

      if (appearance.glow) {
        ctx.shadowColor = `rgba(${r}, ${g}, ${b}, ${busy ? 0.35 : 0.18})`;
        ctx.shadowBlur = busy ? 20 : 12;
      }

      ctx.fillStyle = appearance.dark
        ? 'rgba(19, 27, 42, 0.92)'
        : 'rgba(255, 255, 255, 0.95)';
      ctx.strokeStyle = `rgba(${r}, ${g}, ${b}, 0.55)`;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.roundRect(docX, docY, docW, docH, 10);
      ctx.fill();
      ctx.stroke();
      ctx.shadowBlur = 0;

      // Simulated document content bars (heading, lines, mini chart)
      ctx.fillStyle = `rgba(${r}, ${g}, ${b}, 0.85)`;
      ctx.beginPath();
      ctx.roundRect(docX + 12, docY + 14, 34, 4, 2);
      ctx.fill();

      ctx.fillStyle = appearance.dark
        ? 'rgba(148, 163, 184, 0.38)'
        : 'rgba(100, 116, 139, 0.35)';
      for (let lineIdx = 0; lineIdx < 3; lineIdx++) {
        const lw = lineIdx === 2 ? 38 : 52;
        ctx.beginPath();
        ctx.roundRect(docX + 12, docY + 26 + lineIdx * 10, lw, 3.2, 1.5);
        ctx.fill();
      }

      // Mini bar chart inside page icon
      const bars = [10, 18, 13, 22];
      bars.forEach((bh, idx) => {
        ctx.fillStyle =
          idx === 3
            ? `rgba(${r}, ${g}, ${b}, 0.78)`
            : `rgba(${r}, ${g}, ${b}, 0.34)`;
        ctx.beginPath();
        ctx.roundRect(
          docX + 14 + idx * 12,
          docY + 82 - bh,
          7,
          bh,
          2
        );
        ctx.fill();
      });

      // Active scanning beam
      if (appearance.scan_animation) {
        const beamY = docY + 12 + scanY * (docH - 24);
        const grad = ctx.createLinearGradient(docX, beamY - 8, docX, beamY + 8);
        grad.addColorStop(0, `rgba(${r}, ${g}, ${b}, 0)`);
        grad.addColorStop(0.5, `rgba(${r}, ${g}, ${b}, 0.32)`);
        grad.addColorStop(1, `rgba(${r}, ${g}, ${b}, 0)`);
        ctx.fillStyle = grad;
        ctx.fillRect(docX + 3, beamY - 8, docW - 6, 16);

        ctx.strokeStyle = `rgb(${r}, ${g}, ${b})`;
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.moveTo(docX + 4, beamY);
        ctx.lineTo(docX + docW - 4, beamY);
        ctx.stroke();
      }

      // Completion checkmark badge
      if (completed && !busy) {
        ctx.fillStyle = `rgb(${r}, ${g}, ${b})`;
        ctx.beginPath();
        ctx.arc(docX + docW - 6, docY + docH - 6, 11, 0, Math.PI * 2);
        ctx.fill();

        ctx.strokeStyle = '#0C111C';
        ctx.lineWidth = 2.2;
        ctx.beginPath();
        ctx.moveTo(docX + docW - 11, docY + docH - 6);
        ctx.lineTo(docX + docW - 7, docY + docH - 2);
        ctx.lineTo(docX + docW - 1, docY + docH - 10);
        ctx.stroke();
      }
    };

    draw(performance.now());

    // Respect reduced motion & software WebGL/canvas CPU budgets
    if (!appearance.reduced_motion && (busy || appearance.scan_animation)) {
      const intervalMs = busy ? 50 : 90;
      const timer = window.setInterval(() => {
        draw(performance.now());
      }, intervalMs);
      return () => window.clearInterval(timer);
    }
    return () => cancelAnimationFrame(animId);
  }, [appearance, busy, progress, completed]);

  return (
    <canvas
      ref={canvasRef}
      width={200}
      height={165}
      data-testid="notegen-scan-graphic"
      aria-label="NoteGen live document scanning graphic"
      className="shrink-0 select-none"
    />
  );
};
