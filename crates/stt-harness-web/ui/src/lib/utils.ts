import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** Vendored from atomr-dashboard — Tailwind class merge helper. */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}

/** `83210` ms → `1:23`. */
export function formatTimestamp(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

/** `12.4` → `12.4s`, `185` → `3:05`. */
export function formatDuration(secs: number): string {
  if (secs < 60) return `${secs.toFixed(1)}s`;
  const minutes = Math.floor(secs / 60);
  const seconds = Math.round(secs % 60);
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

// A small fixed palette so each diarized speaker id renders with a
// stable, distinct accent across the list, transcript, and legend.
const SPEAKER_PALETTE = [
  "#3b82f6", // blue
  "#ec4899", // pink
  "#22c55e", // green
  "#f59e0b", // amber
  "#a855f7", // purple
  "#06b6d4", // cyan
  "#ef4444", // red
  "#84cc16", // lime
];

/** Stable accent colour for a numeric speaker id. */
export function speakerColor(id: number): string {
  return SPEAKER_PALETTE[id % SPEAKER_PALETTE.length];
}
