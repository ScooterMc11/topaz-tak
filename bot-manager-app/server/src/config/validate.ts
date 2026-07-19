// Server-side validation — never trust the UI alone. Checks ranges, integer-ness, and the
// GapMin <= GapStd <= GapMax invariant. Returns a flat list of {path, message}; empty = valid.
import {
  AppConfig,
  BotConfig,
  GlobalConfig,
  NumRange,
  RANGES,
  PRIORITY_CLASSES,
  SEARCH_MODES,
  BOT_ROSTER,
} from "./schema.js";

export interface ValidationError {
  path: string;
  message: string;
}

function checkNum(
  errors: ValidationError[],
  path: string,
  value: unknown,
  range: NumRange
): void {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    errors.push({ path, message: "must be a finite number" });
    return;
  }
  if (range.integer && !Number.isInteger(value)) {
    errors.push({ path, message: "must be an integer" });
  }
  if (value < range.min || value > range.max) {
    errors.push({ path, message: `must be between ${range.min} and ${range.max}` });
  }
}

function validateGlobal(errors: ValidationError[], g: GlobalConfig): void {
  if (typeof g.serverHost !== "string" || g.serverHost.trim() === "") {
    errors.push({ path: "global.serverHost", message: "must be a non-empty string" });
  }
  checkNum(errors, "global.port", g.port, RANGES.port);
  if (typeof g.passwordTemplate !== "string") {
    errors.push({ path: "global.passwordTemplate", message: "must be a string" });
  }
  if (typeof g.rated !== "boolean") {
    errors.push({ path: "global.rated", message: "must be a boolean" });
  }
  checkNum(errors, "global.staggerSeconds", g.staggerSeconds, RANGES.staggerSeconds);
  if (typeof g.linkSizes !== "boolean") {
    errors.push({ path: "global.linkSizes", message: "must be a boolean" });
  }
}

function validateBot(errors: ValidationError[], b: BotConfig, idx: number): void {
  const p = `bots[${idx}]`;
  if (typeof b.enabled !== "boolean") {
    errors.push({ path: `${p}.enabled`, message: "must be a boolean" });
  }

  const t = b.tunables ?? ({} as BotConfig["tunables"]);
  if (!SEARCH_MODES.includes(t.searchMode)) {
    errors.push({ path: `${p}.tunables.searchMode`, message: `must be one of ${SEARCH_MODES.join(", ")}` });
  }
  checkNum(errors, `${p}.tunables.depth`, t.depth, RANGES.depth);
  checkNum(errors, `${p}.tunables.nodes`, t.nodes, RANGES.nodes);
  checkNum(errors, `${p}.tunables.gapMin`, t.gapMin, RANGES.gap);
  checkNum(errors, `${p}.tunables.gapStd`, t.gapStd, RANGES.gap);
  checkNum(errors, `${p}.tunables.gapMax`, t.gapMax, RANGES.gap);
  checkNum(errors, `${p}.tunables.biasSlope`, t.biasSlope, RANGES.biasSlope);
  checkNum(errors, `${p}.tunables.movePool`, t.movePool, RANGES.movePool);
  checkNum(errors, `${p}.tunables.alwaysAllow`, t.alwaysAllow, RANGES.alwaysAllow);

  // The invariant, only meaningful once the three are valid numbers.
  if (
    typeof t.gapMin === "number" &&
    typeof t.gapStd === "number" &&
    typeof t.gapMax === "number"
  ) {
    if (!(t.gapMin <= t.gapStd && t.gapStd <= t.gapMax)) {
      errors.push({
        path: `${p}.tunables`,
        message: `GapMin <= GapStd <= GapMax violated (${t.gapMin}/${t.gapStd}/${t.gapMax})`,
      });
    }
  }

  const a = b.advanced ?? ({} as BotConfig["advanced"]);
  if (a.abortDepth != null) checkNum(errors, `${p}.advanced.abortDepth`, a.abortDepth, RANGES.abortDepth);
  if (a.time != null) checkNum(errors, `${p}.advanced.time`, a.time, RANGES.time);
  if (a.hashMb != null) checkNum(errors, `${p}.advanced.hashMb`, a.hashMb, RANGES.hashMb);
  if (a.priority != null && !PRIORITY_CLASSES.includes(a.priority)) {
    errors.push({
      path: `${p}.advanced.priority`,
      message: `must be one of ${PRIORITY_CLASSES.join(", ")}`,
    });
  }
}

export function validateConfig(cfg: AppConfig): ValidationError[] {
  const errors: ValidationError[] = [];

  if (!cfg || typeof cfg !== "object") {
    return [{ path: "", message: "config must be an object" }];
  }
  if (!cfg.global || typeof cfg.global !== "object") {
    errors.push({ path: "global", message: "missing" });
  } else {
    validateGlobal(errors, cfg.global);
  }

  if (!Array.isArray(cfg.bots)) {
    errors.push({ path: "bots", message: "must be an array" });
    return errors;
  }

  // The roster is fixed: exactly these 6 names, each once. Guards against a client dropping/adding.
  const expected = new Set(BOT_ROSTER.map((r) => r.name));
  const seen = new Set<string>();
  cfg.bots.forEach((b, i) => {
    validateBot(errors, b, i);
    if (!expected.has(b.name)) {
      errors.push({ path: `bots[${i}].name`, message: `unknown bot '${b.name}'` });
    } else if (seen.has(b.name)) {
      errors.push({ path: `bots[${i}].name`, message: `duplicate bot '${b.name}'` });
    }
    seen.add(b.name);
  });
  for (const name of expected) {
    if (!seen.has(name)) errors.push({ path: "bots", message: `missing bot '${name}'` });
  }

  return errors;
}
