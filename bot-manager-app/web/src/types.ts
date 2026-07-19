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
  opponent: string | null;
  color: string | null;
  result: string;
  outcome: "win" | "loss" | "draw" | "unknown";
  startedAt: number;
  endedAt: number;
  moveCount: number;
  file: string;
}

export const LEVEL_ORDER: BotLevel[] = ["Advanced", "Intermediate", "Novice"];
