// Opponent ELO enrichment.
//
// The PlayTak telnet protocol never sends ratings — even the official web client fetches them over
// HTTP — so a finished game is stored with ratings_state='pending' and filled in afterwards by this
// worker. Two sources, in order of preference:
//
//   1. GET /v1/games-history/<id>  — the authoritative row for that game: both players' ratings AT
//      THE TIME, plus the rating changes and the unrated flag. Needs the game to have landed in the
//      server's games DB, which lags the end of the game slightly, hence the retry schedule.
//   2. GET /v1/ratings/<opponent>  — the opponent's rating *now*. A fallback for games whose id we
//      never captured (pre-database logs) or that never appear in games-history (e.g. unrated).
//
// Everything here is best-effort: a missing rating leaves the row 'unavailable' and the box plots
// simply have one fewer sample.
import { getDb } from "./db.js";

// PlayTak deployment -> its API base. Mirrors the mapping the official client uses.
const API_BY_HOST: Record<string, string> = {
  "playtak.com": "https://api.playtak.com",
  "www.playtak.com": "https://api.playtak.com",
  "beta.playtak.com": "https://api.beta.playtak.com",
  localhost: "http://localhost:3004",
  "127.0.0.1": "http://localhost:3004",
};

// Deployments to try when a row doesn't know which server it came from (migrated legacy logs). The
// player-name check below makes guessing safe: we only accept a game whose players actually match.
const UNKNOWN_HOST_CANDIDATES = ["playtak.com", "beta.playtak.com"];

// PlayTak stores 0 for "this player has no rating" (guests, brand-new accounts). The games table in
// the official UI treats anything under 100 as absent, so we do too rather than plotting a 0.
const MIN_REAL_RATING = 100;

const MAX_ATTEMPTS = 8;
// Fall back to the current-rating endpoint once games-history has had a few chances to catch up.
const FALLBACK_AFTER_ATTEMPTS = 3;
// Backoff per attempt count: retry soon at first (the game usually lands within a minute), then
// back off to hours so a genuinely missing game costs almost nothing.
const BACKOFF_MS = [0, 30_000, 120_000, 600_000, 1_800_000, 7_200_000, 21_600_000, 86_400_000];

const TICK_MS = 30_000;
const BATCH = 20;
const FETCH_TIMEOUT_MS = 10_000;

interface PendingRow {
  id: number;
  bot_name: string;
  opponent: string | null;
  bot_color: string | null;
  playtak_game_id: string | null;
  server_host: string | null;
  ratings_attempts: number;
}

interface HistoryGame {
  id: number;
  player_white: string;
  player_black: string;
  rating_white: number;
  rating_black: number;
  rating_change_white: number;
  rating_change_black: number;
  unrated: number;
}

const clean = (n: number | undefined | null) =>
  typeof n === "number" && n >= MIN_REAL_RATING ? Math.round(n) : null;

async function getJson<T>(url: string): Promise<T | null> {
  try {
    const res = await fetch(url, { signal: AbortSignal.timeout(FETCH_TIMEOUT_MS) });
    if (!res.ok) return null;
    const body = (await res.json()) as T & { statusCode?: number };
    // The API answers 200 with a { statusCode: 500, message } envelope on some error paths.
    if (body && typeof body === "object" && "statusCode" in body && body.statusCode !== 200)
      return null;
    return body as T;
  } catch {
    return null; // offline / timeout / bad JSON — just try again on the next tick
  }
}

function candidateHosts(row: PendingRow): string[] {
  if (row.server_host) {
    const host = row.server_host.toLowerCase();
    return API_BY_HOST[host] ? [host] : [];
  }
  return UNKNOWN_HOST_CANDIDATES;
}

// Guard against accepting the wrong game: PlayTak game numbers are only unique within a
// deployment, and a local server's numbering restarts from scratch. A row only counts if the bot
// is one of the two players (and the opponent matches, when we recorded one).
function playersMatch(g: HistoryGame, row: PendingRow): boolean {
  const white = (g.player_white ?? "").toLowerCase();
  const black = (g.player_black ?? "").toLowerCase();
  const bot = row.bot_name.toLowerCase();
  if (white !== bot && black !== bot) return false;
  if (!row.opponent) return true;
  const opp = row.opponent.toLowerCase();
  return white === opp || black === opp;
}

// Try the authoritative per-game lookup across this row's candidate deployments.
async function fromHistory(
  row: PendingRow
): Promise<{ host: string; game: HistoryGame } | null> {
  if (!row.playtak_game_id) return null;
  for (const host of candidateHosts(row)) {
    const game = await getJson<HistoryGame>(
      `${API_BY_HOST[host]}/v1/games-history/${encodeURIComponent(row.playtak_game_id)}`
    );
    if (game && typeof game.player_white === "string" && playersMatch(game, row))
      return { host, game };
  }
  return null;
}

async function fromCurrentRating(row: PendingRow): Promise<number | null> {
  if (!row.opponent) return null;
  for (const host of candidateHosts(row)) {
    const body = await getJson<{ rating?: number }>(
      `${API_BY_HOST[host]}/v1/ratings/${encodeURIComponent(row.opponent)}`
    );
    const rating = clean(body?.rating);
    if (rating != null) return rating;
  }
  return null;
}

function markAttempt(row: PendingRow): void {
  const attempts = row.ratings_attempts + 1;
  getDb()
    .prepare(
      `UPDATE games SET ratings_attempts = ?, ratings_checked_at = ?,
        ratings_state = CASE WHEN ? >= ? THEN 'unavailable' ELSE 'pending' END
       WHERE id = ?`
    )
    .run(attempts, Date.now(), attempts, MAX_ATTEMPTS, row.id);
}

// Resolve one pending row. Returns true if it now has a rating.
async function enrichOne(row: PendingRow): Promise<boolean> {
  const hit = await fromHistory(row);
  if (hit) {
    const { host, game } = hit;
    const botIsWhite = game.player_white.toLowerCase() === row.bot_name.toLowerCase();
    getDb()
      .prepare(
        `UPDATE games SET
           opponent_rating = ?, bot_rating = ?, opponent_rating_change = ?, bot_rating_change = ?,
           unrated = ?, ratings_state = 'ok', ratings_source = 'games-history',
           ratings_attempts = ratings_attempts + 1, ratings_checked_at = ?,
           server_host = COALESCE(server_host, ?),
           bot_color = COALESCE(bot_color, ?)
         WHERE id = ?`
      )
      .run(
        clean(botIsWhite ? game.rating_black : game.rating_white),
        clean(botIsWhite ? game.rating_white : game.rating_black),
        botIsWhite ? game.rating_change_black : game.rating_change_white,
        botIsWhite ? game.rating_change_white : game.rating_change_black,
        game.unrated ?? null,
        Date.now(),
        host,
        botIsWhite ? "white" : "black",
        row.id
      );
    return true;
  }

  if (row.ratings_attempts + 1 >= FALLBACK_AFTER_ATTEMPTS) {
    const rating = await fromCurrentRating(row);
    if (rating != null) {
      getDb()
        .prepare(
          `UPDATE games SET opponent_rating = ?, ratings_state = 'ok', ratings_source = 'ratings',
             ratings_attempts = ratings_attempts + 1, ratings_checked_at = ? WHERE id = ?`
        )
        .run(rating, Date.now(), row.id);
      return true;
    }
  }

  markAttempt(row);
  return false;
}

// Pending rows whose backoff has elapsed, newest games first (a game that just finished is the one
// you're most likely watching). The backoff is applied in SQL so a long tail of old, repeatedly
// failed rows can't starve fresh ones out of the batch.
function dueRows(): PendingRow[] {
  const now = Date.now();
  // CASE over the attempt count = the BACKOFF_MS table, expressed for SQLite.
  const backoffSql = BACKOFF_MS.map((ms, i) =>
    i === BACKOFF_MS.length - 1 ? `ELSE ${ms}` : `WHEN ratings_attempts = ${i} THEN ${ms}`
  ).join(" ");
  return getDb()
    .prepare(
      `SELECT id, bot_name, opponent, bot_color, playtak_game_id, server_host, ratings_attempts
         FROM games
        WHERE ratings_state = 'pending'
          AND (ratings_checked_at IS NULL
               OR ? - ratings_checked_at >= (CASE ${backoffSql} END))
        ORDER BY started_at DESC LIMIT ?`
    )
    .all(now, BATCH) as PendingRow[];
}

let timer: NodeJS.Timeout | null = null;
let running = false;

// One pass over the due rows. Sequential on purpose — this is a background chore and there's no
// reason to hammer a public API from a home connection.
export async function runRatingsPass(): Promise<number> {
  if (running) return 0;
  running = true;
  try {
    let filled = 0;
    for (const row of dueRows()) {
      if (await enrichOne(row)) filled++;
    }
    return filled;
  } finally {
    running = false;
  }
}

export function startRatingsWorker(): void {
  if (timer) return;
  const tick = () =>
    void runRatingsPass().catch((e) => console.error("[bot-manager] ratings pass failed:", e));
  tick();
  timer = setInterval(tick, TICK_MS);
  timer.unref();
}

export function stopRatingsWorker(): void {
  if (timer) clearInterval(timer);
  timer = null;
}
