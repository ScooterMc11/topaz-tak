// One-time import of the pre-database game logs (bot-manager-app/game-logs/<Bot>/index.json plus
// the .log files beside it) into SQLite. Idempotent: rows carry their old filename stem as
// legacy_id under a unique index, so re-running this is a no-op and the original files are left
// untouched on disk.
//
// Those entries predate config snapshots, so they land under the LEGACY_CONFIG_HASH sentinel and
// never match a live bot's configuration — exactly right for the ELO box plots, which are supposed
// to describe the config currently in use. The playtak game id and board size are recovered from
// each log's "Game Start" line so the ratings worker can still look their ELOs up.
import { readdir, readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { GAME_LOGS_DIR } from "../bots/paths.js";
import { insertGame } from "./games.js";

// Games imported from before the app recorded configurations. Deliberately not a real hash, so it
// can never collide with a live bot's config.
export const LEGACY_CONFIG_HASH = "legacy";

interface LegacyEntry {
  id: string;
  opponent: string | null;
  color: string | null;
  result: string;
  startedAt: number;
  endedAt: number;
  file: string;
}

// "Game Start <no> <white> vs <black> <color> <size> ..." — the first line of every saved log.
function parseGameStart(lines: string[]): { gameId: string | null; size: number | null } {
  const line = lines.find((l) => l.trim().startsWith("Game Start"));
  if (!line) return { gameId: null, size: null };
  const t = line.trim().split(/\s+/);
  const size = Number(t[7]);
  return { gameId: t[2] ?? null, size: Number.isInteger(size) ? size : null };
}

async function importBot(botName: string): Promise<number> {
  const dir = join(GAME_LOGS_DIR, botName);
  const indexPath = join(dir, "index.json");
  if (!existsSync(indexPath)) return 0;

  let entries: LegacyEntry[];
  try {
    entries = JSON.parse(await readFile(indexPath, "utf8")) as LegacyEntry[];
  } catch {
    console.warn(`[bot-manager] skipping unreadable legacy index for ${botName}`);
    return 0;
  }

  let imported = 0;
  for (const e of entries) {
    if (!e?.id) continue;
    let lines: string[] = [];
    try {
      lines = (await readFile(join(dir, e.file), "utf8")).split(/\r?\n/);
    } catch {
      /* log file pruned or renamed — import the metadata anyway */
    }
    const { gameId, size } = parseGameStart(lines);
    const id = insertGame(
      {
        botName,
        playtakGameId: gameId,
        opponent: e.opponent ?? null,
        color: e.color ?? null,
        result: e.result ?? "",
        startedAt: e.startedAt,
        endedAt: e.endedAt,
        boardSize: size,
        serverHost: null, // unknown — the ratings worker identifies it from the game itself
        serverPort: null,
        rated: null,
        configHash: LEGACY_CONFIG_HASH,
        config: null,
        legacyId: e.id,
      },
      lines
    );
    if (id != null) imported++;
  }
  return imported;
}

// Import every bot's legacy folder. Safe to call on every startup.
export async function migrateLegacyLogs(): Promise<number> {
  if (!existsSync(GAME_LOGS_DIR)) return 0;
  let total = 0;
  const dirents = await readdir(GAME_LOGS_DIR, { withFileTypes: true });
  for (const d of dirents) {
    if (!d.isDirectory()) continue;
    total += await importBot(d.name);
  }
  if (total > 0) console.log(`[bot-manager] imported ${total} legacy game log(s) into the database`);
  return total;
}
