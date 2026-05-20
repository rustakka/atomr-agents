import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** Tailwind class merge helper. */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}

/** Format a unix-millis timestamp as a short local string. */
export function formatRelativeMs(ms: number): string {
  if (!ms) return "—";
  const date = new Date(ms);
  return date.toLocaleString();
}

/** Pretty-print an arbitrary JSON value with 2-space indent. */
export function prettyJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}
