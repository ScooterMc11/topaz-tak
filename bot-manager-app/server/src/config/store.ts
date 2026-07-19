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
  // Schema migration. v3 -> v4 only ADDS the alwaysAllow tunable, so patch it in-place (default 50)
  // and keep every calibrated value. Any other version gap (much older / future) falls back to a
  // reseed rather than guessing. Local config only, so re-tuning is quick.
  if (parsed.version === 3) {
    for (const b of parsed.bots ?? []) {
      if (b.tunables && b.tunables.alwaysAllow == null) b.tunables.alwaysAllow = 50;
    }
    parsed.version = CONFIG_VERSION;
    const errors = validateConfig(parsed);
    if (errors.length) {
      throw new Error(
        `config at ${CONFIG_PATH} failed validation after v3->v4 migration: ` +
          errors.map((e) => `${e.path}: ${e.message}`).join("; ")
      );
    }
    await writeAtomic(parsed);
    cache = parsed;
    console.log(`[bot-manager] migrated config v3 -> v${CONFIG_VERSION} (added alwaysAllow) at ${CONFIG_PATH}`);
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
