// SQLite connection + schema. One file on D: (see paths.ts), opened once for the life of the
// process. better-sqlite3 is synchronous — every query here is a sub-millisecond local read/write,
// so there's nothing to gain from async plumbing and a lot of complexity to lose.
//
// Scaling notes: a game row is ~200 bytes of metadata plus a gzipped log (~2-5 KB for a full 6x6
// game). 100k games ≈ 400 MB — comfortable for SQLite, and the (bot_name, started_at) and
// (bot_name, config_hash) indices keep the two queries the UI makes O(log n).
import Database from "better-sqlite3";
import { gunzipSync } from "node:zlib";
import { DB_PATH, DATA_DIR } from "./paths.js";
import { countUndosAccepted } from "./undos.js";

export type DB = Database.Database;

let db: DB | null = null;

// Bumped whenever the schema changes; `migrate` walks user_version up to this.
const SCHEMA_VERSION = 2;

const SCHEMA_V1 = `
CREATE TABLE IF NOT EXISTS games (
  id                     INTEGER PRIMARY KEY AUTOINCREMENT,
  bot_name               TEXT    NOT NULL,
  -- PlayTak's own game number, from the "Game Start <no> ..." line. Null for games recorded before
  -- we captured it and whose log didn't have it either.
  playtak_game_id        TEXT,
  opponent               TEXT,
  bot_color              TEXT,               -- 'white' | 'black'
  result                 TEXT    NOT NULL,   -- raw PlayTak token: 'R-0', '0-F', '1/2-1/2', ...
  outcome                TEXT    NOT NULL,   -- 'win' | 'loss' | 'draw' | 'unknown' (bot's POV)
  started_at             INTEGER NOT NULL,   -- epoch ms
  ended_at               INTEGER NOT NULL,
  move_count             INTEGER NOT NULL,
  board_size             INTEGER,

  -- Which PlayTak deployment this game was played on. Drives both the ratings lookup and the
  -- PTN Ninja review link. Null on migrated rows until the enricher identifies the server.
  server_host            TEXT,
  server_port            INTEGER,
  rated                  INTEGER,            -- the -Rated flag the bot was launched with (0/1)

  -- The bot's configuration for THIS game, captured at launch. config_hash is what the ELO box
  -- plots match on, so changing any tunable starts a fresh sample.
  config_hash            TEXT    NOT NULL,
  config_json            TEXT,

  -- Ratings, enriched asynchronously from the PlayTak HTTP API (the telnet protocol never sends
  -- them). Null until 'ratings_state' is 'ok'.
  opponent_rating        INTEGER,
  bot_rating             INTEGER,
  opponent_rating_change INTEGER,
  bot_rating_change      INTEGER,
  unrated                INTEGER,            -- authoritative flag from games-history
  ratings_state          TEXT    NOT NULL DEFAULT 'pending', -- 'pending' | 'ok' | 'unavailable'
  ratings_source         TEXT,               -- 'games-history' | 'ratings'
  ratings_attempts       INTEGER NOT NULL DEFAULT 0,
  ratings_checked_at     INTEGER,

  log_gz                 BLOB,               -- gzipped UTF-8 log text
  log_bytes              INTEGER,            -- uncompressed size, for display/debugging
  created_at             INTEGER NOT NULL,
  -- The pre-database filename stem, kept so re-running the legacy import can't duplicate rows.
  legacy_id              TEXT
);

CREATE INDEX IF NOT EXISTS idx_games_bot_started ON games (bot_name, started_at DESC);
CREATE INDEX IF NOT EXISTS idx_games_bot_config  ON games (bot_name, config_hash);
CREATE INDEX IF NOT EXISTS idx_games_ratings     ON games (ratings_state, ratings_attempts);
CREATE INDEX IF NOT EXISTS idx_games_started     ON games (started_at DESC);
CREATE UNIQUE INDEX IF NOT EXISTS ux_games_legacy
  ON games (bot_name, legacy_id) WHERE legacy_id IS NOT NULL;
`;

// v2 adds the per-game count of courtesy undos the opponent accepted. It's derived from the log at
// insert time, so existing rows are backfilled from their stored logs once (see `migrate`).
const SCHEMA_V2 = `
ALTER TABLE games ADD COLUMN undos_accepted INTEGER NOT NULL DEFAULT 0;
`;

function migrate(conn: DB): void {
  const current = conn.pragma("user_version", { simple: true }) as number;
  if (current >= SCHEMA_VERSION) return;
  conn.exec("BEGIN");
  try {
    if (current < 1) conn.exec(SCHEMA_V1);
    if (current < 2) {
      conn.exec(SCHEMA_V2);
      // Only games recorded since the courtesy-undo feature shipped can contain the marker, but the
      // log is the only record of it, so scan them all once rather than guessing a cutoff.
      const rows = conn
        .prepare("SELECT id, log_gz FROM games WHERE log_gz IS NOT NULL")
        .all() as { id: number; log_gz: Buffer }[];
      const upd = conn.prepare("UPDATE games SET undos_accepted = ? WHERE id = ?");
      let touched = 0;
      for (const r of rows) {
        let n = 0;
        try {
          n = countUndosAccepted(gunzipSync(r.log_gz).toString("utf8").split("\n"));
        } catch {
          continue; // a corrupt blob shouldn't block the migration
        }
        if (n > 0) {
          upd.run(n, r.id);
          touched++;
        }
      }
      console.log(
        `[bot-manager] backfilled undos_accepted on ${touched} of ${rows.length} logged games`
      );
    }
    conn.pragma(`user_version = ${SCHEMA_VERSION}`);
    conn.exec("COMMIT");
  } catch (e) {
    conn.exec("ROLLBACK");
    throw e;
  }
  console.log(`[bot-manager] db schema v${current} -> v${SCHEMA_VERSION}`);
}

export function getDb(): DB {
  if (db) return db;
  const conn = new Database(DB_PATH);
  // WAL lets the ratings worker write while the UI reads; NORMAL sync is the right durability
  // trade for a local panel (a power cut can lose the last game, not the database).
  conn.pragma("journal_mode = WAL");
  conn.pragma("synchronous = NORMAL");
  conn.pragma("foreign_keys = ON");
  migrate(conn);
  db = conn;
  console.log(`[bot-manager] game database at ${DB_PATH}`);
  return db;
}

export function closeDb(): void {
  if (!db) return;
  try {
    db.close();
  } catch {
    /* already closed — fine */
  }
  db = null;
}

export { DB_PATH, DATA_DIR };
