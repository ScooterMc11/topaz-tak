// A stable fingerprint of "how this bot plays", used to decide which past games belong to the bot's
// *current* configuration (the ELO box plots match on it).
//
// Only the knobs that change play are included: the tunables (search mode/depth/nodes, drop rate,
// bias, move pool, always-allow) and the advanced overrides (abort depth, time, hash, priority),
// plus board size since a 5x5 and 6x6 bot with identical numbers are not the same player. Global
// settings (server host, rated, stagger) and `enabled` are deliberately excluded — flipping Rated
// or pointing at beta shouldn't wipe the sample.
import { createHash } from "node:crypto";
import { BotConfig, BotTunables, BotAdvanced, BotSize } from "./schema.js";

export interface ConfigSnapshot {
  size: BotSize;
  tunables: BotTunables;
  advanced: BotAdvanced;
}

export function configSnapshot(bot: BotConfig): ConfigSnapshot {
  return { size: bot.size, tunables: { ...bot.tunables }, advanced: { ...bot.advanced } };
}

// JSON.stringify with sorted keys at every level, so key ordering in the config file can never
// change the hash.
function canonical(value: unknown): string {
  if (value === null || typeof value !== "object") return JSON.stringify(value) ?? "null";
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  const entries = Object.entries(value as Record<string, unknown>)
    .filter(([, v]) => v !== undefined)
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  return `{${entries.map(([k, v]) => `${JSON.stringify(k)}:${canonical(v)}`).join(",")}}`;
}

// Short hash — 12 hex chars is 48 bits, far beyond collision range for a handful of local configs,
// and short enough to show in the UI when debugging why a box plot is empty.
export function configHash(snapshot: ConfigSnapshot): string {
  return createHash("sha1").update(canonical(snapshot)).digest("hex").slice(0, 12);
}

export function hashOf(bot: BotConfig): string {
  return configHash(configSnapshot(bot));
}
