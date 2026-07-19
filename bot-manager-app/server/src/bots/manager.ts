// Bot process manager. Spawns run-gembot.ps1 per bot as a managed child (no window), tracks its
// PID + a rolling log buffer, and stops it by killing the whole process tree (taskkill /T) so the
// grandchild topaz.exe can't be orphaned by run-gembot.ps1's restart loop.
import { spawn, ChildProcess } from "node:child_process";
import { AppConfig, BotConfig } from "../config/schema.js";
import { buildGembotArgs } from "./args.js";
import { REPO_ROOT, RUN_GEMBOT } from "./paths.js";
import { recordGame } from "./games.js";
import { loadConfig } from "../config/store.js";

// Large enough to hold a full game's per-move selection logs (~15-20 lines/move) without dropping
// the opening. Bump higher if you run very long games.
const MAX_LOG_LINES = 8000;
const MAX_GAME_LINES = 20000; // per-game capture cap (drops the tail of an absurdly long game)

interface Managed {
  name: string;
  pid: number | undefined;
  startedAt: number;
  child: ChildProcess;
  exited: boolean;
  exitCode: number | null;
  logs: string[];
  lineBuf: string; // partial-line remainder between stdout/stderr chunks
  inGame: boolean; // parsed from the bot's Game Start / Game# Over output
  opponent: string | null; // the other player in the current game
  gameId: string | null;
  gameColor: string | null; // the bot's color this game (for W/L/D)
  gameStartedAt: number | null; // when the current game began
  gameLines: string[]; // captured output for the current game (persisted on Game# Over)
  restartQueued: boolean; // a restart was requested mid-game → auto-restart when this game ends
}

export type BotState = "running" | "exited" | "notRunning";

export interface BotStatus {
  name: string;
  state: BotState;
  pid?: number;
  startedAt?: number;
  uptimeSec?: number;
  exitCode?: number | null;
  inGame?: boolean;
  opponent?: string | null;
  gameId?: string | null;
  restartQueued?: boolean;
}

export interface StartResult {
  name: string;
  result: "starting" | "already-running" | "queued" | "error";
  pid?: number;
  message?: string;
}

export interface StopResult {
  name: string;
  result: "stopped" | "not-running" | "error";
  message?: string;
}

const managed = new Map<string, Managed>();

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function isRunning(name: string): boolean {
  const m = managed.get(name);
  return !!m && !m.exited;
}

function appendOutput(m: Managed, chunk: string): void {
  const text = m.lineBuf + chunk;
  const parts = text.split(/\r?\n/);
  m.lineBuf = parts.pop() ?? ""; // last element is a partial line (or "")
  for (const line of parts) {
    m.logs.push(line);
    trackGameState(m, line);
  }
  if (m.logs.length > MAX_LOG_LINES) m.logs.splice(0, m.logs.length - MAX_LOG_LINES);
}

// Derive in-game status + opponent from the engine's telnet output as it streams by (the Game Start
// line scrolls out of the bounded log buffer during a game, so this must run on ingestion, not by
// re-reading logs). Protocol-4 layout: "Game Start <no> <white> vs <black> <color> <size> ...".
function trackGameState(m: Managed, line: string): void {
  const trimmed = line.trim();
  if (trimmed.startsWith("Game Start")) {
    const t = trimmed.split(/\s+/);
    const white = t[3];
    const black = t[5];
    if (white && black) {
      m.gameId = t[2] ?? null;
      // The bot's own account is one of the two; the opponent is the other (case-insensitive match).
      m.opponent = white.toLowerCase() === m.name.toLowerCase() ? black : white;
      m.gameColor = (t[6] ?? "").toLowerCase() || null; // "white" | "black"
      m.inGame = true;
      m.gameStartedAt = Date.now();
      m.gameLines = [line];
    }
    return;
  }
  if (!m.inGame) return;
  if (m.gameLines.length < MAX_GAME_LINES) m.gameLines.push(line);
  // "Game#<id> Over <result>" ends the game → persist it, then reset the per-game state.
  if (trimmed.startsWith("Game#") && /^Game#\S+\s+Over\b/.test(trimmed)) {
    const result = trimmed.split(/\s+/)[2] ?? "";
    void recordGame(
      m.name,
      {
        opponent: m.opponent,
        color: m.gameColor,
        result,
        startedAt: m.gameStartedAt ?? Date.now(),
        endedAt: Date.now(),
      },
      m.gameLines
    ).catch((e) => console.error(`[bot-manager] failed to save game for ${m.name}:`, e));
    m.inGame = false;
    m.opponent = null;
    m.gameId = null;
    m.gameColor = null;
    m.gameStartedAt = null;
    m.gameLines = [];
    // A restart requested mid-game fires now that the game is over (fresh process, current config).
    if (m.restartQueued) {
      m.restartQueued = false;
      void loadConfig()
        .then((cfg) => restartOne(cfg, m.name))
        .catch((e) => console.error(`[bot-manager] queued restart for ${m.name} failed:`, e));
    }
  }
}

// Spawn one bot. Caller ensures it isn't already running.
function startBot(bot: BotConfig, global: AppConfig["global"]): StartResult {
  const args = [
    "-NoProfile",
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    RUN_GEMBOT,
    ...buildGembotArgs(bot, global),
  ];

  let child: ChildProcess;
  try {
    child = spawn("powershell.exe", args, { cwd: REPO_ROOT, windowsHide: true });
  } catch (e) {
    return { name: bot.name, result: "error", message: (e as Error).message };
  }

  const m: Managed = {
    name: bot.name,
    pid: child.pid,
    startedAt: Date.now(),
    child,
    exited: false,
    exitCode: null,
    logs: [],
    lineBuf: "",
    inGame: false,
    opponent: null,
    gameId: null,
    gameColor: null,
    gameStartedAt: null,
    gameLines: [],
    restartQueued: false,
  };
  managed.set(bot.name, m);

  child.stdout?.setEncoding("utf8");
  child.stderr?.setEncoding("utf8");
  child.stdout?.on("data", (d: string) => appendOutput(m, d));
  child.stderr?.on("data", (d: string) => appendOutput(m, d));
  child.on("error", (err) => {
    appendOutput(m, `[spawn error] ${err.message}\n`);
    m.exited = true;
  });
  child.on("exit", (code) => {
    m.exited = true;
    m.exitCode = code;
    // A process that dies mid-game abandoned it (no result) — discard the partial capture.
    m.inGame = false;
    m.opponent = null;
    m.gameId = null;
    m.gameColor = null;
    m.gameStartedAt = null;
    m.gameLines = [];
    m.restartQueued = false; // the process is gone; nothing to restart-after-game
    appendOutput(m, `[process exited code=${code}]\n`);
  });

  return { name: bot.name, result: "starting", pid: child.pid };
}

// Kill a process tree (the powershell host + its topaz.exe child). Resolves even if the PID is
// already gone (taskkill exits non-zero when the process isn't found — that's fine).
function killTree(pid: number): Promise<void> {
  return new Promise((resolve) => {
    const tk = spawn("taskkill", ["/PID", String(pid), "/T", "/F"], { windowsHide: true });
    tk.on("exit", () => resolve());
    tk.on("error", () => resolve());
  });
}

async function stopBot(name: string): Promise<StopResult> {
  const m = managed.get(name);
  if (!m) return { name, result: "not-running" };
  try {
    if (m.pid != null) await killTree(m.pid);
    managed.delete(name);
    return { name, result: "stopped" };
  } catch (e) {
    return { name, result: "error", message: (e as Error).message };
  }
}

// Start all enabled bots (optionally filtered to `only`), staggered. Returns the plan immediately;
// the staggered launches run in the background so the HTTP call doesn't block for stagger*N seconds.
export function startMany(config: AppConfig, only?: string[]): StartResult[] {
  const targets = config.bots.filter(
    (b) => b.enabled && (!only || only.includes(b.name))
  );
  const plan: StartResult[] = [];
  const toLaunch: BotConfig[] = [];
  for (const b of targets) {
    if (isRunning(b.name)) plan.push({ name: b.name, result: "already-running" });
    else {
      plan.push({ name: b.name, result: "starting" });
      toLaunch.push(b);
    }
  }

  void (async () => {
    for (let i = 0; i < toLaunch.length; i++) {
      if (i > 0) await sleep(config.global.staggerSeconds * 1000);
      if (!isRunning(toLaunch[i].name)) startBot(toLaunch[i], config.global);
    }
  })();

  return plan;
}

export async function stopMany(only?: string[]): Promise<StopResult[]> {
  const names = [...managed.keys()].filter((n) => !only || only.includes(n));
  return Promise.all(names.map((n) => stopBot(n)));
}

// Single-bot controls (used by per-bot routes in Phase 5).
export function startOne(config: AppConfig, name: string): StartResult {
  const bot = config.bots.find((b) => b.name === name);
  if (!bot) return { name, result: "error", message: "unknown bot" };
  if (isRunning(name)) return { name, result: "already-running" };
  return startBot(bot, config.global);
}

export function stopOne(name: string): Promise<StopResult> {
  return stopBot(name);
}

// Restart one bot. If it's mid-game, DON'T cut the game off: flag it to auto-restart when the game
// ends (see trackGameState) and return "queued". Otherwise stop (await the tree kill) + start fresh.
export async function restartOne(config: AppConfig, name: string): Promise<StartResult> {
  const m = managed.get(name);
  if (m && !m.exited && m.inGame) {
    m.restartQueued = true;
    return { name, result: "queued", pid: m.pid };
  }
  await stopBot(name);
  return startOne(config, name);
}

// Restart all (optionally a subset). In-game bots are queued to restart after their current game;
// the rest are stopped and staggered-started now. Never cuts off a game in progress.
export async function restartMany(config: AppConfig, only?: string[]): Promise<StartResult[]> {
  const targets = config.bots.filter((b) => b.enabled && (!only || only.includes(b.name)));
  const queued: StartResult[] = [];
  const restartNow: string[] = [];
  for (const b of targets) {
    const m = managed.get(b.name);
    if (m && !m.exited && m.inGame) {
      m.restartQueued = true;
      queued.push({ name: b.name, result: "queued", pid: m.pid });
    } else {
      restartNow.push(b.name);
    }
  }
  await stopMany(restartNow);
  return [...queued, ...startMany(config, restartNow)];
}

export function getStatus(allNames: string[]): BotStatus[] {
  return allNames.map((name) => {
    const m = managed.get(name);
    if (!m) return { name, state: "notRunning" };
    if (m.exited) return { name, state: "exited", exitCode: m.exitCode, pid: m.pid };
    return {
      name,
      state: "running",
      pid: m.pid,
      startedAt: m.startedAt,
      uptimeSec: Math.floor((Date.now() - m.startedAt) / 1000),
      inGame: m.inGame,
      opponent: m.opponent,
      gameId: m.gameId,
      restartQueued: m.restartQueued,
    };
  });
}

export function getLogs(name: string, tail = 200): string[] {
  const m = managed.get(name);
  if (!m) return [];
  return m.logs.slice(-tail);
}

// Host PIDs of live managed bots, so status enrichment can tell "our" processes from orphans.
export function listManaged(): { name: string; pid: number | undefined; exited: boolean }[] {
  return [...managed.values()].map((m) => ({ name: m.name, pid: m.pid, exited: m.exited }));
}

// Force-kill arbitrary process trees (used to clean up orphans this panel didn't launch).
export async function forceKill(pids: number[]): Promise<void> {
  await Promise.all(pids.map((pid) => killTree(pid)));
}

// Best-effort cleanup so bots don't outlive the backend on a graceful shutdown (Ctrl-C, tsx-watch
// reload). A hard crash can still orphan them — Phase 6 adds orphan reconciliation.
export async function shutdown(): Promise<void> {
  await stopMany();
}
