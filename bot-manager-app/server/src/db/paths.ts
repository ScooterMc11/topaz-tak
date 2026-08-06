// Where the game database lives. It grows without bound (we never prune games), so it defaults to
// the roomy D: drive rather than the app folder on C:. Resolution order:
//   1. BOT_MANAGER_DATA_DIR   — explicit override (any drive/path)
//   2. D:\TakBotManager       — the default when D: exists and is writable
//   3. <bot-manager-app>/data — last-resort fallback so the app still runs on a machine without D:
import { existsSync, mkdirSync, accessSync, constants } from "node:fs";
import { resolve } from "node:path";
import { APP_ROOT } from "../bots/paths.js";

const PREFERRED_DRIVE = "D:\\";
const PREFERRED_DIR = resolve(PREFERRED_DRIVE, "TakBotManager");
const FALLBACK_DIR = resolve(APP_ROOT, "data");

// Usable = we can create it and write into it. A drive that exists but is read-only (or a mounted
// volume that's currently offline) must not take the app down, so every probe is best-effort.
function usable(dir: string): boolean {
  try {
    mkdirSync(dir, { recursive: true });
    accessSync(dir, constants.W_OK);
    return true;
  } catch {
    return false;
  }
}

function resolveDataDir(): string {
  const override = process.env.BOT_MANAGER_DATA_DIR?.trim();
  if (override) {
    if (usable(override)) return resolve(override);
    console.warn(`[bot-manager] BOT_MANAGER_DATA_DIR '${override}' isn't writable; falling back`);
  }
  if (existsSync(PREFERRED_DRIVE) && usable(PREFERRED_DIR)) return PREFERRED_DIR;
  if (usable(FALLBACK_DIR)) return FALLBACK_DIR;
  throw new Error(`no writable data directory (tried ${PREFERRED_DIR} and ${FALLBACK_DIR})`);
}

export const DATA_DIR = resolveDataDir();
export const DB_PATH = resolve(DATA_DIR, "bot-games.db");
