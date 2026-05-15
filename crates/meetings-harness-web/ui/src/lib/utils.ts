import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** Tailwind class merge helper. */
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

/** Format a unix-millis timestamp as a short relative-ish string. */
export function formatRelativeMs(ms: number): string {
  if (!ms) return "—";
  const date = new Date(ms);
  return date.toLocaleString();
}

const ATTENDEE_PALETTE = [
  "#3b82f6", "#ec4899", "#22c55e", "#f59e0b",
  "#a855f7", "#06b6d4", "#ef4444", "#84cc16",
];

/** Stable accent color for an attendee — by primary speaker_tag. */
export function attendeeColor(speakerTag: number): string {
  return ATTENDEE_PALETTE[speakerTag % ATTENDEE_PALETTE.length];
}
