// Bot lifecycle + status routes. Phase 4: start/stop all (with optional `only` subset) and a basic
// managed-PID status. Per-bot start/stop/restart and richer status land in Phase 5.
import { Router } from "express";
import { loadConfig } from "../config/store.js";
import { BOT_ROSTER } from "../config/schema.js";
import {
  startMany,
  stopMany,
  restartMany,
  startOne,
  stopOne,
  restartOne,
  getStatus,
  getLogs,
  listManaged,
  forceKill,
} from "../bots/manager.js";
import { scanSystem } from "../bots/scan.js";
import { listGames, listAllGames, getGame, getGameText, eloStats } from "../db/games.js";
import { hashOf, configSnapshot } from "../config/hash.js";

export const botsRouter = Router();

const rosterNames = () => BOT_ROSTER.map((b) => b.name);

// Start all enabled bots, or a subset via { "only": ["AdvancedBot6", ...] }. Returns the launch
// plan immediately; staggered launches proceed in the background (watch status/logs for progress).
botsRouter.post("/start", async (req, res, next) => {
  try {
    const only = Array.isArray(req.body?.only) ? (req.body.only as string[]) : undefined;
    const config = await loadConfig();
    res.json({ started: startMany(config, only) });
  } catch (e) {
    next(e);
  }
});

// Stop all managed bots, or a subset via { "only": [...] }.
botsRouter.post("/stop", async (req, res, next) => {
  try {
    const only = Array.isArray(req.body?.only) ? (req.body.only as string[]) : undefined;
    res.json({ stopped: await stopMany(only) });
  } catch (e) {
    next(e);
  }
});

// Restart all enabled bots (or a subset): stop everything managed, then staggered start.
botsRouter.post("/restart", async (req, res, next) => {
  try {
    const only = Array.isArray(req.body?.only) ? (req.body.only as string[]) : undefined;
    const config = await loadConfig();
    res.json({ started: await restartMany(config, only) });
  } catch (e) {
    next(e);
  }
});

// Kill every bot process this panel didn't launch (unmanaged run-gembot hosts + their topaz, plus
// any stray topaz). Lets you clean up leftovers from run-ladder.ps1 windows without hunting PIDs.
// Registered before the /:name/* routes so "orphans" isn't captured as a bot name.
botsRouter.post("/orphans/stop", async (_req, res, next) => {
  try {
    const config = await loadConfig();
    const scan = await scanSystem(config.global.port);
    const managedHostPids = new Set(listManaged().filter((m) => !m.exited).map((m) => m.pid));
    const hostPids = scan.hosts.filter((h) => !managedHostPids.has(h.pid)).map((h) => h.pid);
    const topazPids = scan.topaz.filter((t) => !managedHostPids.has(t.ppid)).map((t) => t.pid);
    const pids = [...new Set([...hostPids, ...topazPids])];
    await forceKill(pids);
    res.json({ killed: pids });
  } catch (e) {
    next(e);
  }
});

// Per-bot controls.
botsRouter.post("/:name/start", async (req, res, next) => {
  try {
    const config = await loadConfig();
    res.json(startOne(config, req.params.name));
  } catch (e) {
    next(e);
  }
});

botsRouter.post("/:name/stop", async (req, res, next) => {
  try {
    res.json(await stopOne(req.params.name));
  } catch (e) {
    next(e);
  }
});

botsRouter.post("/:name/restart", async (req, res, next) => {
  try {
    const config = await loadConfig();
    res.json(await restartOne(config, req.params.name));
  } catch (e) {
    next(e);
  }
});

// Enriched status: managed state + a system scan that adds "connected" (bot's topaz has an
// ESTABLISHED connection to the server port) and "orphan" (a bot running outside this panel).
botsRouter.get("/status", async (_req, res, next) => {
  try {
    const config = await loadConfig();
    const base = getStatus(rosterNames());
    const scan = await scanSystem(config.global.port);

    const live = listManaged().filter((m) => !m.exited);
    const managedHostPids = new Set(live.map((m) => m.pid));
    const connSet = new Set(scan.conns);
    const enginePidFor = (hostPid?: number) =>
      hostPid == null ? undefined : scan.topaz.find((t) => t.ppid === hostPid)?.pid;

    // Unmanaged run-gembot hosts, keyed by the bot username they launched.
    const orphanHosts = scan.hosts.filter((h) => !managedHostPids.has(h.pid));
    const orphanHostByUser = new Map(orphanHosts.map((h) => [h.user, h.pid]));

    const bots = base.map((b) => {
      if (b.state === "running") {
        const enginePid = enginePidFor(b.pid);
        const connected = enginePid != null && connSet.has(enginePid);
        return { ...b, state: connected ? "connected" : "running", enginePid, connected };
      }
      // Not managed-running: is there an unmanaged host for this bot? => orphan.
      const hostPid = orphanHostByUser.get(b.name);
      if (hostPid != null) {
        const enginePid = enginePidFor(hostPid);
        const connected = enginePid != null && connSet.has(enginePid);
        return { ...b, state: "orphan", pid: hostPid, enginePid, connected };
      }
      return b;
    });

    // Orphan topaz.exe = engines whose parent host isn't managed by us.
    const orphanTopaz = scan.topaz.filter((t) => !managedHostPids.has(t.ppid)).map((t) => t.pid);

    res.json({ bots, orphans: { hosts: orphanHosts, topazPids: orphanTopaz } });
  } catch (e) {
    next(e);
  }
});

botsRouter.get("/:name/logs", (req, res) => {
  const tail = Number(req.query.tail) || 200;
  res.json({ name: req.params.name, lines: getLogs(req.params.name, tail) });
});

// How many games the UI is allowed to pull at once. The database keeps everything; these caps just
// stop a stray ?limit=999999 from serializing the entire history into one response.
const MAX_LIMIT = 500;
const clampLimit = (raw: unknown, fallback: number) => {
  const n = Number(raw);
  return Number.isFinite(n) && n > 0 ? Math.min(Math.floor(n), MAX_LIMIT) : fallback;
};

// Combined newest-first history across every bot (the History card). Registered before the
// /:name/* routes so "games" can't be read as a bot name.
botsRouter.get("/games", (req, res, next) => {
  try {
    res.json({ games: listAllGames(clampLimit(req.query.limit, 50)) });
  } catch (e) {
    next(e);
  }
});

// Saved game history for one bot, newest first. The UI asks for 30; the database keeps them all.
botsRouter.get("/:name/games", (req, res, next) => {
  try {
    res.json({
      name: req.params.name,
      games: listGames(req.params.name, clampLimit(req.query.limit, 30)),
    });
  } catch (e) {
    next(e);
  }
});

// Opponent-ELO distributions for the bot's CURRENT saved configuration. A config the bot has never
// played under simply returns empty arrays — that's the intended "reset" when tunables change.
botsRouter.get("/:name/elo-stats", async (req, res, next) => {
  try {
    const config = await loadConfig();
    const bot = config.bots.find((b) => b.name === req.params.name);
    if (!bot) return res.status(404).json({ error: "unknown bot" });
    res.json({ name: bot.name, config: configSnapshot(bot), ...eloStats(bot.name, hashOf(bot)) });
  } catch (e) {
    next(e);
  }
});

// The saved log as plain text, so a game is still shareable as a file now that logs live in the
// database rather than beside it on disk.
botsRouter.get("/:name/games/:id/log.txt", (req, res, next) => {
  try {
    const text = getGameText(req.params.name, req.params.id);
    if (text == null) return res.status(404).json({ error: "game not found" });
    res.type("text/plain").send(text);
  } catch (e) {
    next(e);
  }
});

// One game: its log plus the configuration the bot was running at the time and a review link.
botsRouter.get("/:name/games/:id", (req, res, next) => {
  try {
    const game = getGame(req.params.name, req.params.id);
    if (!game) return res.status(404).json({ error: "game not found" });
    res.json(game);
  } catch (e) {
    next(e);
  }
});
