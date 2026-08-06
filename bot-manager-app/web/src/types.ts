// Config types — mirror the server's config/schema.ts. (Kept in sync manually; the server also
// serves the field ranges via GET /api/config/schema so slider bounds come from one source.)

export type BotLevel = "Advanced" | "Intermediate" | "Novice";
export type BotSize = 5 | 6;
export type PriorityClass = "Idle" | "BelowNormal" | "Normal" | "AboveNormal" | "High";

export type SearchMode = "depth" | "nodes";

export interface BotTunables {
  searchMode: SearchMode;
  depth: number;
  nodes: number;
  gapMin: number;
  gapStd: number;
  gapMax: number;
  biasSlope: number;
  movePool: number;
  alwaysAllow: number;
}

export interface BotAdvanced {
  abortDepth: number | null;
  time: number | null;
  hashMb: number | null;
  priority: PriorityClass | null;
}

export interface BotConfig {
  name: string;
  size: BotSize;
  level: BotLevel;
  enabled: boolean;
  tunables: BotTunables;
  advanced: BotAdvanced;
}

export interface GlobalConfig {
  serverHost: string;
  port: number;
  passwordTemplate: string;
  rated: boolean;
  staggerSeconds: number;
  linkSizes: boolean;
}

export interface AppConfig {
  version: number;
  global: GlobalConfig;
  bots: BotConfig[];
}

export interface NumRange {
  min: number;
  max: number;
  step: number;
  integer: boolean;
}

export interface Stage {
  name: string;
  host: string;
  port: number;
}

export interface ConfigSchema {
  version: number;
  ranges: {
    depth: NumRange;
    nodes: NumRange;
    gap: NumRange;
    biasSlope: NumRange;
    movePool: NumRange;
    alwaysAllow: NumRange;
    staggerSeconds: NumRange;
    port: NumRange;
    abortDepth: NumRange;
    time: NumRange;
    hashMb: NumRange;
  };
  roster: { name: string; size: BotSize; level: BotLevel }[];
  priorities: PriorityClass[];
  advancedDefaults: {
    abortDepth: number;
    timeBySize: Record<string, number>;
    hashMb: number;
    priority: PriorityClass;
  };
  stages: Stage[];
}

export type BotState = "connected" | "running" | "orphan" | "exited" | "notRunning";

export interface BotStatus {
  name: string;
  state: BotState;
  pid?: number;
  startedAt?: number;
  uptimeSec?: number;
  exitCode?: number | null;
  enginePid?: number;
  connected?: boolean;
  inGame?: boolean;
  opponent?: string | null;
  gameId?: string | null;
  restartQueued?: boolean;
}

export interface OrphanHost {
  pid: number;
  user: string;
}

export interface StatusResponse {
  bots: BotStatus[];
  orphans: { hosts: OrphanHost[]; topazPids: number[] };
}

export interface GameSummary {
  id: string;
  botName: string;
  playtakGameId: string | null;
  opponent: string | null;
  opponentRating: number | null;
  botRating: number | null;
  color: string | null;
  result: string; // raw PlayTak token, e.g. "0-F", "R-0", "1/2-1/2"
  outcome: "win" | "loss" | "draw" | "unknown";
  startedAt: number;
  endedAt: number;
  moveCount: number;
  boardSize: number | null;
  rated: boolean | null;
  ratingsState: "pending" | "ok" | "unavailable";
  reviewUrl: string | null; // PTN Ninja deep link, null for games on a non-public server
}

// The configuration a bot was running for one particular game (mirrors the server's ConfigSnapshot).
export interface ConfigSnapshot {
  size: BotSize;
  tunables: BotTunables;
  advanced: BotAdvanced;
}

export interface GameDetail extends GameSummary {
  config: ConfigSnapshot | null;
  configHash: string;
  serverHost: string | null;
  lines: string[];
}

// Opponent-ELO distributions for a bot's *current* configuration. `won`/`lost` are ascending
// opponent ratings; a config that's never been played returns both empty.
export interface EloStats {
  name: string;
  config: ConfigSnapshot;
  configHash: string;
  won: number[];
  lost: number[];
  matchedGames: number;
  totalGames: number;
  pendingRatings: number;
}

export const LEVEL_ORDER: BotLevel[] = ["Advanced", "Intermediate", "Novice"];
