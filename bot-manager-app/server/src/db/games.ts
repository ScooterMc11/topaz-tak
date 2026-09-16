// Game history store. Every finished game becomes one row in the SQLite database (see db.ts),
// including its gzipped log text and a snapshot of the bot's configuration at launch. Nothing is
// pruned — the UI just asks for the newest N.
import { gzipSync, gunzipSync } from "node:zlib";
import { getDb } from "./db.js";
import { countUndosAccepted } from "./undos.js";
import type { ConfigSnapshot } from "../config/hash.js";

export interface GameSummary {
  id: string; // the row id, as a string (the API keys games by it)
  botName: string;
  playtakGameId: string | null;
  opponent: string | null;
  opponentRating: number | null;
  botRating: number | null;
  color: string | null; // the bot's color this game
  result: string; // raw PlayTak result token, e.g. "R-0", "0-F", "1/2-1/2"
  outcome: "win" | "loss" | "draw" | "unknown"; // from the bot's perspective
  startedAt: number;
  endedAt: number;
  moveCount: number;
  // Courtesy undos the opponent accepted this game. Counted from the log, so it's 0 for games
  // played before the feature and for bots with it switched off.
  undosAccepted: number;
  boardSize: number | null;
  rated: boolean | null;
  ratingsState: "pending" | "ok" | "unavailable";
  reviewUrl: string | null; // PTN Ninja deep link, when the game exists on a public PlayTak
}

export interface GameDetail extends GameSummary {
  config: ConfigSnapshot | null;
  configHash: string;
  serverHost: string | null;
  lines: string[];
}

// What the manager hands us when a game ends.
export interface GameRecord {
  botName: string;
  playtakGameId: string | null;
  opponent: string | null;
  color: string | null;
  result: string;
  startedAt: number;
  endedAt: number;
  boardSize: number | null;
  serverHost: string | null;
  serverPort: number | null;
  rated: boolean | null;
  configHash: string;
  config: ConfigSnapshot | null;
  legacyId?: string | null;
}

interface GameRow {
  id: number;
  bot_name: string;
  playtak_game_id: string | null;
  opponent: string | null;
  bot_color: string | null;
  result: string;
  outcome: string;
  started_at: number;
  ended_at: number;
  move_count: number;
  undos_accepted: number;
  board_size: number | null;
  server_host: string | null;
  server_port: number | null;
  rated: number | null;
  config_hash: string;
  config_json: string | null;
  opponent_rating: number | null;
  bot_rating: number | null;
  opponent_rating_change: number | null;
  bot_rating_change: number | null;
  unrated: number | null;
  ratings_state: string;
  ratings_source: string | null;
  ratings_attempts: number;
  ratings_checked_at: number | null;
  log_gz: Buffer | null;
  log_bytes: number | null;
  created_at: number;
  legacy_id: string | null;
}

// Result is "<white>-<black>"; the non-"0" side won (1/2 => draw). Map to the bot's own outcome.
export function computeOutcome(color: string | null, result: string): GameSummary["outcome"] {
  if (!result) return "unknown";
  if (result.includes("1/2")) return "draw";
  // Anything that isn't a two-sided score token ("Abandoned", say) is nobody's win — without this
  // the split below reads the whole token as White's side and scores it as a White victory.
  if (!/^\S+-\S+$/.test(result)) return "unknown";
  const [w, b] = result.split("-");
  const whiteWon = !!w && w !== "0";
  const blackWon = !!b && b !== "0";
  if (whiteWon === blackWon) return "unknown"; // both/neither → can't tell
  if (color === "white") return whiteWon ? "win" : "loss";
  if (color === "black") return blackWon ? "win" : "loss";
  return "unknown";
}

// The public game-history site for a PlayTak deployment, or null when the game only exists on a
// server nobody else can reach (a local dev server). `/games/<id>/ninjaviewer` is the same route
// the playtak-games table's "Review → PTN Ninja" link uses; it redirects into ptn.ninja.
export function reviewUrl(serverHost: string | null, playtakGameId: string | null): string | null {
  if (!playtakGameId || !serverHost) return null;
  const host = serverHost.toLowerCase();
  if (host === "playtak.com" || host === "www.playtak.com")
    return `https://playtak.com/games/${playtakGameId}/ninjaviewer`;
  if (host === "beta.playtak.com")
    return `https://beta.playtak.com/games/${playtakGameId}/ninjaviewer`;
  return null; // localhost / anything unrecognised — no public review page
}

function toSummary(r: GameRow): GameSummary {
  return {
    id: String(r.id),
    botName: r.bot_name,
    playtakGameId: r.playtak_game_id,
    opponent: r.opponent,
    opponentRating: r.opponent_rating,
    botRating: r.bot_rating,
    color: r.bot_color,
    result: r.result,
    outcome: r.outcome as GameSummary["outcome"],
    startedAt: r.started_at,
    endedAt: r.ended_at,
    moveCount: r.move_count,
    undosAccepted: r.undos_accepted ?? 0,
    boardSize: r.board_size,
    rated: r.rated == null ? null : !!r.rated,
    ratingsState: r.ratings_state as GameSummary["ratingsState"],
    reviewUrl: reviewUrl(r.server_host, r.playtak_game_id),
  };
}

const SUMMARY_COLS = `id, bot_name, playtak_game_id, opponent, bot_color, result, outcome,
  started_at, ended_at, move_count, undos_accepted, board_size, server_host, rated,
  opponent_rating, bot_rating, ratings_state`;

// Insert a finished game. Returns the new row id (or null if a legacy import hit its unique index).
export function insertGame(rec: GameRecord, lines: string[]): number | null {
  const text = lines.join("\n") + "\n";
  const gz = gzipSync(Buffer.from(text, "utf8"));
  const info = getDb()
    .prepare(
      `INSERT OR IGNORE INTO games (
         bot_name, playtak_game_id, opponent, bot_color, result, outcome, started_at, ended_at,
         move_count, undos_accepted, board_size, server_host, server_port, rated, config_hash,
         config_json, log_gz, log_bytes, created_at, legacy_id
       ) VALUES (
         @bot_name, @playtak_game_id, @opponent, @bot_color, @result, @outcome, @started_at,
         @ended_at, @move_count, @undos_accepted, @board_size, @server_host, @server_port, @rated,
         @config_hash, @config_json, @log_gz, @log_bytes, @created_at, @legacy_id
       )`
    )
    .run({
      bot_name: rec.botName,
      playtak_game_id: rec.playtakGameId,
      opponent: rec.opponent,
      bot_color: rec.color,
      result: rec.result,
      outcome: computeOutcome(rec.color, rec.result),
      started_at: rec.startedAt,
      ended_at: rec.endedAt,
      move_count: lines.filter((l) => l.startsWith("========== MOVE")).length,
      undos_accepted: countUndosAccepted(lines),
      board_size: rec.boardSize,
      server_host: rec.serverHost,
      server_port: rec.serverPort,
      rated: rec.rated == null ? null : rec.rated ? 1 : 0,
      config_hash: rec.configHash,
      config_json: rec.config ? JSON.stringify(rec.config) : null,
      log_gz: gz,
      log_bytes: Buffer.byteLength(text, "utf8"),
      created_at: Date.now(),
      legacy_id: rec.legacyId ?? null,
    });
  return info.changes > 0 ? Number(info.lastInsertRowid) : null;
}

export function listGames(botName: string, limit = 30): GameSummary[] {
  const rows = getDb()
    .prepare(
      `SELECT ${SUMMARY_COLS} FROM games WHERE bot_name = ? ORDER BY started_at DESC LIMIT ?`
    )
    .all(botName, limit) as GameRow[];
  return rows.map(toSummary);
}

// Newest games across every bot, for the combined History card.
export function listAllGames(limit = 50): GameSummary[] {
  const rows = getDb()
    .prepare(`SELECT ${SUMMARY_COLS} FROM games ORDER BY started_at DESC LIMIT ?`)
    .all(limit) as GameRow[];
  return rows.map(toSummary);
}

export function getGame(botName: string, id: string): GameDetail | null {
  const rowId = Number(id);
  if (!Number.isInteger(rowId)) return null;
  const r = getDb()
    .prepare(`SELECT * FROM games WHERE id = ? AND bot_name = ?`)
    .get(rowId, botName) as GameRow | undefined;
  if (!r) return null;
  let config: ConfigSnapshot | null = null;
  if (r.config_json) {
    try {
      config = JSON.parse(r.config_json) as ConfigSnapshot;
    } catch {
      config = null; // corrupt snapshot shouldn't hide the log
    }
  }
  return {
    ...toSummary(r),
    config,
    configHash: r.config_hash,
    serverHost: r.server_host,
    lines: r.log_gz ? gunzipSync(r.log_gz).toString("utf8").split(/\r?\n/) : [],
  };
}

export function getGameText(botName: string, id: string): string | null {
  const rowId = Number(id);
  if (!Number.isInteger(rowId)) return null;
  const r = getDb()
    .prepare(`SELECT log_gz FROM games WHERE id = ? AND bot_name = ?`)
    .get(rowId, botName) as { log_gz: Buffer | null } | undefined;
  if (!r) return null;
  return r.log_gz ? gunzipSync(r.log_gz).toString("utf8") : "";
}

export interface EloStats {
  configHash: string;
  won: number[]; // opponent ELOs of games this config won, ascending
  lost: number[]; // ...and lost
  matchedGames: number; // games on this config that count (any outcome, rated or not)
  totalGames: number; // games for this bot across every config
  pendingRatings: number; // counted games still waiting on an ELO lookup
  walkovers: number; // games on this config dropped by NOT_A_WALKOVER
}

// PlayTak reports a game decided by resignation, timeout or a disconnect as a plain '1-0'/'0-1';
// games actually played out end in a road ('R-0'/'0-R') or flat ('F-0'/'0-F') token instead. When
// one of those default endings arrives before the bot has taken board_size moves, nobody really
// played the game — the opponent sat down, made a move or two, and left — so it says nothing about
// the bot's strength and the ELO plots leave it out. `move_count` is the bot's own turns (one per
// "========== MOVE" marker in the log), i.e. moves rather than plies, which is the unit the board
// size is compared against. A row with no recorded board size can't be judged, so it's kept.
const NOT_A_WALKOVER = `(board_size IS NULL OR result NOT IN ('1-0', '0-1') OR move_count >= board_size)`;

// Opponent ELOs split by outcome, restricted to games played on `configHash`. Draws are excluded
// (the box plots are specifically won-against vs lost-against), as are games with no known rating
// and the walkovers described above.
export function eloStats(botName: string, hash: string): EloStats {
  const db = getDb();
  const pick = (outcome: "win" | "loss") =>
    (
      db
        .prepare(
          `SELECT opponent_rating AS r FROM games
            WHERE bot_name = ? AND config_hash = ? AND outcome = ? AND opponent_rating IS NOT NULL
              AND ${NOT_A_WALKOVER}
            ORDER BY opponent_rating`
        )
        .all(botName, hash, outcome) as { r: number }[]
    ).map((x) => x.r);

  const count = (sql: string, ...args: unknown[]) =>
    (db.prepare(sql).get(...args) as { n: number }).n;

  return {
    configHash: hash,
    won: pick("win"),
    lost: pick("loss"),
    matchedGames: count(
      `SELECT COUNT(*) AS n FROM games
        WHERE bot_name = ? AND config_hash = ? AND ${NOT_A_WALKOVER}`,
      botName,
      hash
    ),
    totalGames: count(`SELECT COUNT(*) AS n FROM games WHERE bot_name = ?`, botName),
    pendingRatings: count(
      `SELECT COUNT(*) AS n FROM games
        WHERE bot_name = ? AND config_hash = ? AND ratings_state = 'pending'
          AND ${NOT_A_WALKOVER}`,
      botName,
      hash
    ),
    walkovers: count(
      `SELECT COUNT(*) AS n FROM games
        WHERE bot_name = ? AND config_hash = ? AND NOT ${NOT_A_WALKOVER}`,
      botName,
      hash
    ),
  };
}

export function countGames(): number {
  return (getDb().prepare(`SELECT COUNT(*) AS n FROM games`).get() as { n: number }).n;
}
