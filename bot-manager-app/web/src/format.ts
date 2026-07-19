// Small formatting helpers shared by the per-bot Logs drawer and the combined History card.
import type { GameSummary } from "./types";

export function fmtDateTime(ms: number): string {
  return new Date(ms).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function fmtDuration(startedAt: number, endedAt: number): string {
  const s = Math.max(0, Math.floor((endedAt - startedAt) / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  return m < 60 ? `${m}m` : `${Math.floor(m / 60)}h ${m % 60}m`;
}

export const OUTCOME_COLOR: Record<GameSummary["outcome"], "success" | "error" | "default" | "warning"> = {
  win: "success",
  loss: "error",
  draw: "warning",
  unknown: "default",
};
