// Resolve repo-relative paths. The server lives at bot-manager-app/server/src/bots; the repo root
// (where run-gembot.ps1 lives) is four levels up.
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
// src/bots -> src -> server -> bot-manager-app -> repo root
export const APP_ROOT = resolve(__dirname, "..", "..", ".."); // bot-manager-app/
export const REPO_ROOT = resolve(APP_ROOT, "..");
export const RUN_GEMBOT = resolve(REPO_ROOT, "run-gembot.ps1");
// Per-bot saved game logs (last 20 games each): bot-manager-app/game-logs/<BotName>/
export const GAME_LOGS_DIR = resolve(APP_ROOT, "game-logs");
