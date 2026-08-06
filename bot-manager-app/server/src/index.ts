// Bot Manager backend — Express server bound to 127.0.0.1 (local, single-user, no auth).
// Config + bot process management.
import express, { NextFunction, Request, Response } from "express";
import { configRouter } from "./routes/config.js";
import { botsRouter } from "./routes/bots.js";
import { shutdown } from "./bots/manager.js";
import { getDb, closeDb, DB_PATH } from "./db/db.js";
import { countGames } from "./db/games.js";
import { migrateLegacyLogs } from "./db/migrateLegacy.js";
import { startRatingsWorker, stopRatingsWorker } from "./db/ratings.js";

const HOST = "127.0.0.1";
const PORT = 4317;

// Open the game database before anything can serve a request, import any pre-database logs, then
// let the ELO worker start chewing through games whose ratings aren't known yet.
getDb();
void migrateLegacyLogs()
  .catch((e) => console.error("[bot-manager] legacy log import failed:", e))
  .finally(() => startRatingsWorker());

const app = express();
app.use(express.json());

// Liveness probe the frontend polls to show the "backend up" dot.
app.get("/api/health", (_req, res) => {
  res.json({
    ok: true,
    service: "bot-manager-server",
    version: "0.1.0",
    db: { path: DB_PATH, games: countGames() },
    now: Date.now(),
  });
});

app.use("/api/config", configRouter);
app.use("/api/bots", botsRouter);

// Central error handler — turns thrown errors (e.g. corrupt config file) into 500 JSON.
app.use((err: unknown, _req: Request, res: Response, _next: NextFunction) => {
  const message = err instanceof Error ? err.message : String(err);
  console.error("[bot-manager] error:", message);
  res.status(500).json({ error: message });
});

const server = app.listen(PORT, HOST, () => {
  console.log(`[bot-manager] server listening on http://${HOST}:${PORT}`);
});

// Stop managed bots before exiting (Ctrl-C / tsx-watch reload) so they don't outlive the backend.
let shuttingDown = false;
async function gracefulExit(signal: string) {
  if (shuttingDown) return;
  shuttingDown = true;
  console.log(`[bot-manager] ${signal} — stopping bots and shutting down`);
  stopRatingsWorker();
  await shutdown();
  closeDb();
  server.close(() => process.exit(0));
  // Fallback if close hangs.
  setTimeout(() => process.exit(0), 3000).unref();
}
process.on("SIGINT", () => void gracefulExit("SIGINT"));
process.on("SIGTERM", () => void gracefulExit("SIGTERM"));
