// Thin fetch wrappers for the backend API (same-origin via the Vite /api proxy).
import type {
  AppConfig,
  ConfigSchema,
  EloStats,
  GameDetail,
  GameSummary,
  StatusResponse,
} from "./types";

async function getJson<T>(url: string): Promise<T> {
  const res = await fetch(url);
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error(body?.error ?? `${res.status} ${res.statusText}`);
  }
  return res.json() as Promise<T>;
}

// Validation errors from the server come back as { errors: [{path, message}] }.
async function sendJson<T>(method: string, url: string, body: unknown): Promise<T> {
  const res = await fetch(url, {
    method,
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    const b = await res.json().catch(() => ({}));
    if (Array.isArray(b?.errors)) {
      throw new Error(b.errors.map((e: { path: string; message: string }) => `${e.path}: ${e.message}`).join("; "));
    }
    throw new Error(b?.error ?? `${res.status} ${res.statusText}`);
  }
  return res.json() as Promise<T>;
}

export const api = {
  health: () => getJson<{ ok: boolean }>("/api/health"),
  getConfig: () => getJson<AppConfig>("/api/config"),
  getSchema: () => getJson<ConfigSchema>("/api/config/schema"),
  putConfig: (cfg: AppConfig) => sendJson<AppConfig>("PUT", "/api/config", cfg),

  getStatus: () => getJson<StatusResponse>("/api/bots/status"),
  startAll: (only?: string[]) => sendJson("POST", "/api/bots/start", only ? { only } : {}),
  stopAll: (only?: string[]) => sendJson("POST", "/api/bots/stop", only ? { only } : {}),
  restartAll: () => sendJson("POST", "/api/bots/restart", {}),
  startBot: (name: string) => sendJson("POST", `/api/bots/${name}/start`, {}),
  stopBot: (name: string) => sendJson("POST", `/api/bots/${name}/stop`, {}),
  restartBot: (name: string) => sendJson("POST", `/api/bots/${name}/restart`, {}),
  stopOrphans: () => sendJson<{ killed: number[] }>("POST", "/api/bots/orphans/stop", {}),
  getLogs: (name: string, tail = 200) =>
    getJson<{ name: string; lines: string[] }>(`/api/bots/${name}/logs?tail=${tail}`),
  // Per-bot history, newest first. The database keeps every game; the UI only asks for a window.
  getGames: (name: string, limit = 30) =>
    getJson<{ name: string; games: GameSummary[] }>(`/api/bots/${name}/games?limit=${limit}`),
  // Combined newest-first history across all bots (one request instead of one per bot).
  getAllGames: (limit = 50) => getJson<{ games: GameSummary[] }>(`/api/bots/games?limit=${limit}`),
  getGame: (name: string, id: string) =>
    getJson<GameDetail>(`/api/bots/${name}/games/${encodeURIComponent(id)}`),
  // Opponent ELOs won/lost against, restricted to the bot's current saved configuration.
  getEloStats: (name: string) => getJson<EloStats>(`/api/bots/${name}/elo-stats`),
};
