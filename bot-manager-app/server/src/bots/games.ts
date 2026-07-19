// Per-bot saved game logs. Each finished game's captured output is written to a plain-text .log file
// under game-logs/<BotName>/ (shareable as-is), catalogued in that bot's index.json. Only the newest
// MAX_GAMES_PER_BOT games are kept; older logs are pruned.
import { resolve, join } from "node:path";
import { mkdir, writeFile, rename, readFile, unlink } from "node:fs/promises";
import { existsSync } from "node:fs";
import { GAME_LOGS_DIR } from "./paths.js";

const MAX_GAMES_PER_BOT = 20;

export interface GameSummary {
  id: string; // unique key = the .log filename stem
  opponent: string | null;
  color: string | null; // the bot's color this game ("white" | "black")
  result: string; // raw PlayTak result token, e.g. "R-0", "0-F", "1/2-1/2"
  outcome: "win" | "loss" | "draw" | "unknown"; // from the bot's perspective
  startedAt: number;
  endedAt: number;
  moveCount: number;
  file: string; // .log filename (relative to the bot's dir)
}

export interface GameRecordMeta {
  opponent: string | null;
  color: string | null;
  result: string;
  startedAt: number;
  endedAt: number;
}

const sanitize = (s: string) => s.replace(/[^A-Za-z0-9_.-]/g, "_");
const botDir = (botName: string) => resolve(GAME_LOGS_DIR, sanitize(botName));
const indexPath = (botName: string) => join(botDir(botName), "index.json");

async function readIndex(botName: string): Promise<GameSummary[]> {
  const p = indexPath(botName);
  if (!existsSync(p)) return [];
  try {
    return JSON.parse(await readFile(p, "utf8")) as GameSummary[];
  } catch {
    return [];
  }
}

async function writeIndex(botName: string, games: GameSummary[]): Promise<void> {
  const dir = botDir(botName);
  await mkdir(dir, { recursive: true });
  const tmp = join(dir, "index.json.tmp");
  await writeFile(tmp, JSON.stringify(games, null, 2) + "\n", "utf8");
  await rename(tmp, indexPath(botName));
}

// Result is "<white>-<black>"; the non-"0" side won (1/2 => draw). Map to the bot's own outcome.
function computeOutcome(color: string | null, result: string): GameSummary["outcome"] {
  if (!result) return "unknown";
  if (result.includes("1/2")) return "draw";
  const [w, b] = result.split("-");
  const whiteWon = !!w && w !== "0";
  const blackWon = !!b && b !== "0";
  if (whiteWon === blackWon) return "unknown"; // both/neither → can't tell
  if (color === "white") return whiteWon ? "win" : "loss";
  if (color === "black") return blackWon ? "win" : "loss";
  return "unknown";
}

export async function recordGame(
  botName: string,
  meta: GameRecordMeta,
  lines: string[]
): Promise<void> {
  const dir = botDir(botName);
  await mkdir(dir, { recursive: true });

  const iso = new Date(meta.startedAt).toISOString().replace(/[:.]/g, "-");
  const id = `${iso}_vs_${sanitize(meta.opponent ?? "unknown")}_${sanitize(meta.result || "na")}`;
  const file = `${id}.log`;

  const tmp = join(dir, `${file}.tmp`);
  await writeFile(tmp, lines.join("\n") + "\n", "utf8");
  await rename(tmp, join(dir, file));

  const summary: GameSummary = {
    id,
    opponent: meta.opponent,
    color: meta.color,
    result: meta.result,
    outcome: computeOutcome(meta.color, meta.result),
    startedAt: meta.startedAt,
    endedAt: meta.endedAt,
    moveCount: lines.filter((l) => l.startsWith("========== MOVE")).length,
    file,
  };

  const games = await readIndex(botName);
  games.unshift(summary); // newest first
  const keep = games.slice(0, MAX_GAMES_PER_BOT);
  const drop = games.slice(MAX_GAMES_PER_BOT);
  await writeIndex(botName, keep);
  for (const g of drop) {
    try {
      await unlink(join(dir, g.file));
    } catch {
      /* already gone — fine */
    }
  }
}

export async function listGames(botName: string): Promise<GameSummary[]> {
  return readIndex(botName);
}

// Serve only ids present in the index (prevents path traversal and stale-file reads).
export async function readGameLog(botName: string, id: string): Promise<string[] | null> {
  const game = (await readIndex(botName)).find((g) => g.id === id);
  if (!game) return null;
  const p = join(botDir(botName), game.file);
  if (!existsSync(p)) return null;
  return (await readFile(p, "utf8")).split(/\r?\n/);
}
