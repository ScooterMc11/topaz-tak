// Seed config — dynamic gap triples per level, BiasSlope 8, MovePool 16, and a fixed search DEPTH
// per level (same for both sizes): Novice 4, Intermediate 6, Advanced 8. Advanced fields are null
// (fall back to run-gembot.ps1 defaults). Global mirrors run-ladder.ps1 param defaults
// (localhost:10000, unrated, stagger 8).
import {
  AppConfig,
  BotConfig,
  BOT_ROSTER,
  CONFIG_VERSION,
  BotLevel,
  BotSize,
} from "./schema.js";

// gap triple + fixed search depth per level (both same for 5x5 and 6x6).
const GAP_BY_LEVEL: Record<BotLevel, { gapMin: number; gapStd: number; gapMax: number }> = {
  Advanced: { gapMin: 20, gapStd: 30, gapMax: 40 },
  Intermediate: { gapMin: 40, gapStd: 50, gapMax: 60 },
  Novice: { gapMin: 60, gapStd: 80, gapMax: 100 },
};

const DEPTH_BY_LEVEL: Record<BotLevel, number> = {
  Advanced: 8,
  Intermediate: 6,
  Novice: 4,
};

// Per-size node caps for the alternative "nodes" mode (the pre-depth ladder values).
const NODES_BY_SIZE: Record<BotSize, number> = { 5: 40000, 6: 50000 };

function seedBot(name: string, size: BotSize, level: BotLevel): BotConfig {
  const gap = GAP_BY_LEVEL[level];
  return {
    name,
    size,
    level,
    enabled: true,
    offerUndos: true,
    tunables: {
      searchMode: "depth", // default to fixed-depth; switch per bot in the UI
      depth: DEPTH_BY_LEVEL[level],
      nodes: NODES_BY_SIZE[size],
      gapMin: gap.gapMin,
      gapStd: gap.gapStd,
      gapMax: gap.gapMax,
      biasSlope: 8,
      movePool: 16,
      alwaysAllow: 50,
    },
    advanced: { abortDepth: null, time: null, hashMb: null, priority: null },
  };
}

export function defaultConfig(): AppConfig {
  return {
    version: CONFIG_VERSION,
    global: {
      serverHost: "localhost",
      port: 10000,
      passwordTemplate: "password",
      rated: false,
      staggerSeconds: 8,
      linkSizes: true,
    },
    bots: BOT_ROSTER.map((b) => seedBot(b.name, b.size, b.level)),
  };
}
