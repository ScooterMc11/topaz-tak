// Config schema: types, the fixed bot roster, and the field ranges that drive both server-side
// validation and (via GET /api/config/schema) the frontend sliders. One source of truth.

export type BotLevel = "Advanced" | "Intermediate" | "Novice";
export type BotSize = 5 | 6;
export type PriorityClass = "Idle" | "BelowNormal" | "Normal" | "AboveNormal" | "High";
// Which strength knob is active for a bot: a fixed search depth, or a per-move node cap.
export type SearchMode = "depth" | "nodes";
export const SEARCH_MODES: SearchMode[] = ["depth", "nodes"];

export const PRIORITY_CLASSES: PriorityClass[] = [
  "Idle",
  "BelowNormal",
  "Normal",
  "AboveNormal",
  "High",
];

export interface BotTunables {
  searchMode: SearchMode; // which knob is active: "depth" (fixed) or "nodes" (per-move cap)
  depth: number; // fixed iterative-deepening search depth (used when searchMode === "depth")
  nodes: number; // per-move node cap (used when searchMode === "nodes")
  gapMin: number;
  gapStd: number;
  gapMax: number;
  biasSlope: number;
  movePool: number;
  alwaysAllow: number; // absolute cp floor: moves scoring >= this stay eligible outside the drop-rate band
}

// Advanced per-bot overrides. null => omit the arg so run-gembot.ps1's own default applies.
export interface BotAdvanced {
  abortDepth: number | null;
  time: number | null;
  hashMb: number | null;
  priority: PriorityClass | null;
}

export interface BotConfig {
  name: string; // fixed identity
  size: BotSize; // fixed
  level: BotLevel; // fixed (grouping + size link)
  enabled: boolean;
  // Courtesy undo: offer to take back an opponent move that hung an instant road when they had a
  // non-losing alternative. Deliberately NOT a tunable — it changes manners, not strength, so it
  // stays out of the config hash and never splits a bot's ELO sample (see hash.ts).
  offerUndos: boolean;
  tunables: BotTunables;
  advanced: BotAdvanced;
}

// How many courtesy undos a bot offers per game, by level: the weaker the bot, the more likely its
// opponent is a beginner who'd rather take the move back. Passed to the engine as -UndoOffers;
// `offerUndos: false` sends 0 instead, which turns the behaviour off outright.
export const UNDO_OFFERS_BY_LEVEL: Record<BotLevel, number> = {
  Novice: 3,
  Intermediate: 2,
  Advanced: 1,
};

export interface GlobalConfig {
  serverHost: string;
  port: number;
  passwordTemplate: string; // "{user}" -> each bot's username
  rated: boolean;
  staggerSeconds: number;
  linkSizes: boolean; // UI: edit a level's 6x6 & 5x5 tunables together (mirrors all but nodes)
}

export interface AppConfig {
  version: number;
  global: GlobalConfig;
  bots: BotConfig[];
}

export const CONFIG_VERSION = 5;

// --- Field ranges (min/max/step). Consumed by validation and served to the frontend. ---
export interface NumRange {
  min: number;
  max: number;
  step: number;
  integer: boolean;
}

export const RANGES = {
  depth: { min: 1, max: 12, step: 1, integer: true },
  nodes: { min: 1000, max: 1000000, step: 1000, integer: true },
  gap: { min: 0, max: 200, step: 5, integer: true }, // shared by gapMin/gapStd/gapMax ("Drop rate")
  biasSlope: { min: 0, max: 20, step: 0.5, integer: false },
  movePool: { min: 1, max: 16, step: 1, integer: true },
  alwaysAllow: { min: -100, max: 200, step: 5, integer: true },
  staggerSeconds: { min: 0, max: 30, step: 1, integer: true },
  port: { min: 1, max: 65535, step: 1, integer: true },
  abortDepth: { min: 1, max: 6, step: 1, integer: true },
  time: { min: 60, max: 1200, step: 30, integer: true },
  hashMb: { min: 16, max: 512, step: 16, integer: true },
} as const satisfies Record<string, NumRange>;

// The 6 bots are a fixed roster (identity = name + size + level). Only tunables/advanced/enabled edit.
export interface BotIdentity {
  name: string;
  size: BotSize;
  level: BotLevel;
}

export const BOT_ROSTER: BotIdentity[] = [
  { name: "AdvancedBot6", size: 6, level: "Advanced" },
  { name: "IntermediateBot6", size: 6, level: "Intermediate" },
  { name: "NoviceBot6", size: 6, level: "Novice" },
  { name: "AdvancedBot5", size: 5, level: "Advanced" },
  { name: "IntermediateBot5", size: 5, level: "Intermediate" },
  { name: "NoviceBot5", size: 5, level: "Novice" },
];

// Server stage presets (playtak-api README). The UI offers these as one-click host+port targets.
export interface Stage {
  name: string;
  host: string;
  port: number;
}

export const STAGES: Stage[] = [
  { name: "Local", host: "localhost", port: 10000 },
  { name: "Beta", host: "beta.playtak.com", port: 10002 },
  { name: "Prod", host: "playtak.com", port: 10000 },
];

// run-gembot.ps1 defaults, surfaced so the UI can show "(default N)" next to advanced overrides.
export const ADVANCED_DEFAULTS = {
  abortDepth: 1,
  timeBySize: { 5: 420, 6: 600 } as Record<BotSize, number>,
  hashMb: 48,
  priority: "BelowNormal" as PriorityClass,
};
