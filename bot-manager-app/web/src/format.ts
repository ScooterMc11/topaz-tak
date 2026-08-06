// Small formatting helpers shared by the per-bot Logs drawer and the combined History card.
import type { ConfigSnapshot, GameSummary } from "./types";

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

// The raw PlayTak result token as shown in the UI. "1/2-1/2" is what a draw looks like on the wire;
// the saved filenames sanitised it to "1_2-1_2", so accept either.
export function fmtResult(result: string): string {
  if (!result) return "—";
  return result.replace(/1_2/g, "½").replace(/1\/2/g, "½");
}

// Break a game's stored configuration into short "label: value" pairs for the log footer. Advanced
// overrides only appear when the bot actually set one (null = run-gembot.ps1's own default).
export function configChips(c: ConfigSnapshot): { label: string; value: string }[] {
  const t = c.tunables;
  const a = c.advanced;
  const chips = [
    { label: "Board", value: `${c.size}×${c.size}` },
    t.searchMode === "depth"
      ? { label: "Depth", value: String(t.depth) }
      : { label: "Nodes", value: t.nodes.toLocaleString() },
    { label: "Drop rate", value: `${t.gapMin} / ${t.gapStd} / ${t.gapMax}` },
    { label: "Always allow", value: `${t.alwaysAllow >= 0 ? "+" : ""}${t.alwaysAllow} cp` },
    { label: "Bias slope", value: String(t.biasSlope) },
    { label: "Move pool", value: String(t.movePool) },
  ];
  if (a.abortDepth != null) chips.push({ label: "Abort depth", value: String(a.abortDepth) });
  if (a.time != null) chips.push({ label: "Time", value: `${a.time}s` });
  if (a.hashMb != null) chips.push({ label: "Hash", value: `${a.hashMb} MB` });
  if (a.priority != null) chips.push({ label: "Priority", value: a.priority });
  return chips;
}
