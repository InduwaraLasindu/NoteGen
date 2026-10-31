import { Appearance } from './types';

export interface Scene {
  scan: number;
  orbit: number;
  pulse: number;
  reveal: number;
  check: number;
}

export function motionEnabled(style: Appearance): boolean {
  return !style.reduced_motion && Number.isFinite(style.motion) && style.motion > 0;
}

export function transitionSeconds(style: Appearance): number {
  if (motionEnabled(style)) {
    return Math.min(style.transition_ms, 1000) / 1000 / Math.min(3, Math.max(0.1, style.motion));
  }
  return 0;
}

function easeOut(value: number): number {
  const clamped = Math.min(1, Math.max(0, value));
  return 1 - Math.pow(1 - clamped, 3);
}

export function computeScene(
  elapsed: number,
  style: Appearance,
  busy: boolean,
  paused: boolean,
  completedAge: number | null
): Scene {
  if (!motionEnabled(style) || paused) {
    return {
      scan: 0.5,
      orbit: 0,
      pulse: 0.5,
      reveal: 1,
      check: completedAge !== null ? 1 : 0,
    };
  }
  const seconds = Number.isFinite(elapsed) ? Math.max(0, elapsed) : 0;
  const speed = Math.min(3, Math.max(0.1, style.motion));
  const time = (seconds % 86400) * speed;
  const period = busy ? 2.4 : 4.8;
  const angle = ((time % period) / period) * Math.PI * 2;
  return {
    scan: (1 - Math.cos(angle)) * 0.5,
    orbit: (time % 1000) * 0.12,
    pulse: (1 + Math.sin(time * 2.2)) * 0.5,
    reveal: style.intro_animation ? easeOut((seconds / 0.55) * speed) : 1,
    check:
      completedAge !== null
        ? easeOut(Number.isFinite(completedAge) ? (Math.max(0, completedAge) * speed) / 0.45 : 1)
        : 0,
  };
}
