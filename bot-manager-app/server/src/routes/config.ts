// Config REST routes: GET/PUT full config, PATCH global, PATCH a single bot, and GET the schema
// (ranges/roster/defaults) the frontend renders sliders from. All writes re-validate the WHOLE
// config server-side before persisting.
import { Router } from "express";
import {
  AppConfig,
  BotConfig,
  BotTunables,
  BotAdvanced,
  GlobalConfig,
  RANGES,
  BOT_ROSTER,
  PRIORITY_CLASSES,
  ADVANCED_DEFAULTS,
  STAGES,
  CONFIG_VERSION,
} from "../config/schema.js";
import { validateConfig } from "../config/validate.js";
import { loadConfig, saveConfig } from "../config/store.js";

export const configRouter = Router();

// Static schema for the frontend (field ranges, fixed roster, advanced defaults, enums).
configRouter.get("/schema", (_req, res) => {
  res.json({
    version: CONFIG_VERSION,
    ranges: RANGES,
    roster: BOT_ROSTER,
    priorities: PRIORITY_CLASSES,
    advancedDefaults: ADVANCED_DEFAULTS,
    stages: STAGES,
  });
});

configRouter.get("/", async (_req, res, next) => {
  try {
    res.json(await loadConfig());
  } catch (e) {
    next(e);
  }
});

// Replace the whole config. Validates before writing; 400 with the error list on failure.
configRouter.put("/", async (req, res, next) => {
  try {
    const incoming = req.body as AppConfig;
    const errors = validateConfig(incoming);
    if (errors.length) return res.status(400).json({ errors });
    // Normalize the version stamp; identity fields are trusted post-validation.
    incoming.version = CONFIG_VERSION;
    await saveConfig(incoming);
    res.json(incoming);
  } catch (e) {
    next(e);
  }
});

// Partial update of global settings (only known keys are applied).
configRouter.patch("/global", async (req, res, next) => {
  try {
    const cfg = structuredClone(await loadConfig());
    const patch = (req.body ?? {}) as Partial<GlobalConfig>;
    const g = cfg.global;
    const keys: (keyof GlobalConfig)[] = [
      "serverHost",
      "port",
      "passwordTemplate",
      "rated",
      "staggerSeconds",
      "linkSizes",
    ];
    for (const k of keys) {
      if (k in patch && patch[k] !== undefined) (g[k] as unknown) = patch[k];
    }
    const errors = validateConfig(cfg);
    if (errors.length) return res.status(400).json({ errors });
    await saveConfig(cfg);
    res.json(cfg);
  } catch (e) {
    next(e);
  }
});

// Partial update of one bot's tunables/advanced/enabled. Identity (name/size/level) is immutable.
configRouter.patch("/bots/:name", async (req, res, next) => {
  try {
    const cfg = structuredClone(await loadConfig());
    const bot = cfg.bots.find((b) => b.name === req.params.name);
    if (!bot) return res.status(404).json({ error: `unknown bot '${req.params.name}'` });

    const patch = (req.body ?? {}) as Partial<
      Pick<BotConfig, "enabled"> & { tunables: Partial<BotTunables>; advanced: Partial<BotAdvanced> }
    >;

    if (typeof patch.enabled === "boolean") bot.enabled = patch.enabled;

    if (patch.tunables) {
      const tKeys: (keyof BotTunables)[] = [
        "searchMode",
        "depth",
        "nodes",
        "gapMin",
        "gapStd",
        "gapMax",
        "biasSlope",
        "movePool",
        "alwaysAllow",
      ];
      for (const k of tKeys) {
        const v = patch.tunables[k];
        if (v !== undefined) (bot.tunables[k] as unknown) = v;
      }
    }

    if (patch.advanced) {
      const aKeys: (keyof BotAdvanced)[] = ["abortDepth", "time", "hashMb", "priority"];
      for (const k of aKeys) {
        if (k in patch.advanced) (bot.advanced[k] as unknown) = patch.advanced[k];
      }
    }

    const errors = validateConfig(cfg);
    if (errors.length) return res.status(400).json({ errors });
    await saveConfig(cfg);
    res.json(bot);
  } catch (e) {
    next(e);
  }
});
