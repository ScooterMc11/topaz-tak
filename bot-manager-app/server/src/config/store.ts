// Config persistence. The file lives at bot-manager-app/bot-manager-config.json (git-ignored; parent
// of the server dir). Atomic write = temp file + rename so a crash mid-write can't corrupt it.
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { readFile, writeFile, rename, mkdir } from "node:fs/promises";
import { existsSync } from "node:fs";
import { AppConfig, CONFIG_VERSION } from "./schema.js";
import { defaultConfig } from "./defaults.js";
import { validateConfig } from "./validate.js";

const __dirname = dirname(fileURLToPath(import.meta.url));
// src/config -> up to bot-manager-app/, where the config file lives.
export const CONFIG_PATH = resolve(__dirname, "..", "..", "..", "bot-manager-config.json");

let cache: AppConfig | null = null;

async function writeAtomic(cfg: AppConfig): Promise<void> {
  await mkdir(dirname(CONFIG_PATH), { recursive: true });
  const tmp = `${CONFIG_PATH}.tmp`;
  await writeFile(tmp, JSON.stringify(cfg, null, 2) + "\n", "utf8");
  await rename(tmp, CONFIG_PATH);
}

// Load from disk, seeding defaults on first run. A present-but-invalid file is a hard error
// (surfaced to the caller) rather than silently overwritten.
export async function loadConfig(): Promise<AppConfig> {
  if (cache) return cache;

  if (!existsSync(CONFIG_PATH)) {
    const seed = defaultConfig();
    await writeAtomic(seed);
    cache = seed;
    console.log(`[bot-manager] seeded config at ${CONFIG_PATH}`);
    return seed;
  }

  const raw = await readFile(CONFIG_PATH, "utf8");
  let parsed: AppConfig;
  try {
    parsed = JSON.parse(raw) as AppConfig;
  } catch (e) {
    throw new Error(`config at ${CONFIG_PATH} is not valid JSON: ${(e as Error).message}`);
  }
  // Schema migrations. Each step only ADDS a field, so patch it in-place and keep every calibrated
  // value: v3 -> v4 adds the alwaysAllow tunable (default 50), v4 -> v5 the offerUndos switch
  // (default on), v5 -> v6 the per-bot undoOfferLimit (default null = unlimited). A config several
  // steps behind walks them all. Any other version gap (much older / future) falls back to a
  // reseed rather than guessing. Local config only, so re-tuning is quick.
  if (parsed.version === 3 || parsed.version === 4 || parsed.version === 5) {
    const from = parsed.version;
    for (const b of parsed.bots ?? []) {
      if (from === 3 && b.tunables && b.tunables.alwaysAllow == null) b.tunables.alwaysAllow = 50;
      if (b.offerUndos == null) b.offerUndos = true;
      // null is a meaningful value here (unlimited), so only fill in a genuinely absent field.
      if (b.undoOfferLimit === undefined) b.undoOfferLimit = null;
    }
    parsed.version = CONFIG_VERSION;
    const errors = validateConfig(parsed);
    if (errors.length) {
      throw new Error(
        `config at ${CONFIG_PATH} failed validation after v${from}->v${CONFIG_VERSION} migration: ` +
          errors.map((e) => `${e.path}: ${e.message}`).join("; ")
      );
    }
    await writeAtomic(parsed);
    cache = parsed;
    console.log(
      `[bot-manager] migrated config v${from} -> v${CONFIG_VERSION} at ${CONFIG_PATH}`
    );
    return parsed;
  }
  if (parsed.version !== CONFIG_VERSION) {
    const seed = defaultConfig();
    await writeAtomic(seed);
    cache = seed;
    console.log(
      `[bot-manager] config was v${parsed.version}; reseeded to v${CONFIG_VERSION} at ${CONFIG_PATH}`
    );
    return seed;
  }
  const errors = validateConfig(parsed);
  if (errors.length) {
    throw new Error(
      `config at ${CONFIG_PATH} failed validation: ` +
        errors.map((e) => `${e.path}: ${e.message}`).join("; ")
    );
  }
  cache = parsed;
  return parsed;
}

// Persist an already-validated config and update the cache.
export async function saveConfig(cfg: AppConfig): Promise<void> {
  await writeAtomic(cfg);
  cache = cfg;
}

export function getCached(): AppConfig | null {
  return cache;
}
