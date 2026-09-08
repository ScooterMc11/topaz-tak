import { useEffect, useMemo, useState } from "react";
import AppBar from "@mui/material/AppBar";
import Toolbar from "@mui/material/Toolbar";
import Typography from "@mui/material/Typography";
import Box from "@mui/material/Box";
import Container from "@mui/material/Container";
import Tooltip from "@mui/material/Tooltip";
import Button from "@mui/material/Button";
import Chip from "@mui/material/Chip";
import Alert from "@mui/material/Alert";
import Snackbar from "@mui/material/Snackbar";
import CircularProgress from "@mui/material/CircularProgress";
import useMediaQuery from "@mui/material/useMediaQuery";
import { useTheme } from "@mui/material/styles";
import CircleIcon from "@mui/icons-material/Circle";
import PlayArrowIcon from "@mui/icons-material/PlayArrow";
import StopIcon from "@mui/icons-material/Stop";
import RestartAltIcon from "@mui/icons-material/RestartAlt";
import SaveIcon from "@mui/icons-material/Save";
import UndoIcon from "@mui/icons-material/Undo";
import { api } from "./api";
import type {
  AppConfig,
  BotAdvanced,
  BotStatus,
  BotTunables,
  ConfigSchema,
  GlobalConfig,
  OrphanHost,
} from "./types";
import { LEVEL_ORDER } from "./types";
import GlobalPanel from "./components/GlobalPanel";
import BotCard from "./components/BotCard";
import LogsDrawer from "./components/LogsDrawer";
import type { LogsTarget } from "./components/LogsDrawer";
import HistoryCard from "./components/HistoryCard";

type HealthState = "checking" | "up" | "down";

function useBackendHealth(intervalMs = 3000): HealthState {
  const [state, setState] = useState<HealthState>("checking");
  useEffect(() => {
    let cancelled = false;
    async function check() {
      try {
        const body = await api.health();
        if (!cancelled) setState(body?.ok ? "up" : "down");
      } catch {
        if (!cancelled) setState("down");
      }
    }
    check();
    const id = setInterval(check, intervalMs);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [intervalMs]);
  return state;
}

const GAME_TIMES_KEY = "botManager.gameStartTimes";

const HEALTH_META: Record<HealthState, { color: string; label: string }> = {
  checking: { color: "#9e9e9e", label: "Checking backend…" },
  up: { color: "#4caf50", label: "Backend connected" },
  down: { color: "#f44336", label: "Backend unreachable" },
};

export default function App() {
  const health = useBackendHealth();
  const meta = HEALTH_META[health];
  const theme = useTheme();
  const compact = useMediaQuery(theme.breakpoints.down("sm")); // small screens: tighter primary buttons

  const [baseline, setBaseline] = useState<AppConfig | null>(null); // last saved
  const [draft, setDraft] = useState<AppConfig | null>(null); // editable
  const [schema, setSchema] = useState<ConfigSchema | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [toast, setToast] = useState<{ msg: string; severity: "success" | "error" } | null>(null);
  const [statuses, setStatuses] = useState<Record<string, BotStatus>>({});
  const [orphans, setOrphans] = useState<OrphanHost[]>([]);
  const [logsTarget, setLogsTarget] = useState<LogsTarget | null>(null);
  // Bumped on every successful save. The Logs drawer's ELO box plots match on the bot's saved
  // configuration, so they need to re-query the moment that configuration changes.
  const [configVersion, setConfigVersion] = useState(0);
  // Client-observed game start times, keyed by `${botName}:${gameId}` and persisted to localStorage
  // so the in-game duration survives page reloads (the backend doesn't track a game-start timestamp).
  const [gameStartTimes, setGameStartTimes] = useState<Record<string, number>>(() => {
    try {
      return JSON.parse(localStorage.getItem(GAME_TIMES_KEY) ?? "{}");
    } catch {
      return {};
    }
  });

  useEffect(() => {
    Promise.all([api.getConfig(), api.getSchema()])
      .then(([cfg, sch]) => {
        setBaseline(cfg);
        setDraft(structuredClone(cfg));
        setSchema(sch);
      })
      .catch((e) => setLoadError(String(e.message ?? e)));
  }, []);

  async function refreshStatus() {
    try {
      const { bots, orphans } = await api.getStatus();
      setStatuses(Object.fromEntries(bots.map((b) => [b.name, b])));
      setOrphans(orphans.hosts);
      // Record a start time the first time we see each active game; prune games that have ended.
      setGameStartTimes((prev) => {
        const next: Record<string, number> = {};
        let changed = false;
        for (const b of bots) {
          if (b.inGame && b.gameId) {
            const key = `${b.name}:${b.gameId}`;
            next[key] = prev[key] ?? Date.now();
            if (prev[key] === undefined) changed = true;
          }
        }
        // Anything in prev but not in next has ended → dropping it counts as a change.
        if (!changed && Object.keys(prev).length !== Object.keys(next).length) changed = true;
        if (!changed) return prev;
        localStorage.setItem(GAME_TIMES_KEY, JSON.stringify(next));
        return next;
      });
    } catch {
      setStatuses({}); // backend unreachable → dots go grey
      setOrphans([]);
    }
  }

  // Poll bot status so dots/uptime track reality (staggered launches appear over successive ticks).
  useEffect(() => {
    refreshStatus();
    const id = setInterval(refreshStatus, 2500);
    return () => clearInterval(id);
  }, []);

  // Run a lifecycle action, toast the outcome, and refresh status immediately.
  async function runAction(fn: () => Promise<unknown>, okMsg: string) {
    try {
      await fn();
      setToast({ msg: okMsg, severity: "success" });
    } catch (e) {
      setToast({ msg: `${okMsg} failed — ${(e as Error).message}`, severity: "error" });
    } finally {
      refreshStatus();
    }
  }

  const dirty = useMemo(
    () => !!draft && !!baseline && JSON.stringify(draft) !== JSON.stringify(baseline),
    [draft, baseline]
  );

  // Rough RAM budget: sum the hash tables of enabled bots (the one config knob that reserves memory).
  // The search working set adds more, so warn well below physical RAM. hashMb default is 48.
  const RAM_WARN_MB = 1024;
  const hashTotalMb = useMemo(() => {
    if (!draft || !schema) return 0;
    return draft.bots
      .filter((b) => b.enabled)
      .reduce((sum, b) => sum + (b.advanced.hashMb ?? schema.advancedDefaults.hashMb), 0);
  }, [draft, schema]);

  // --- edit handlers ---
  function updateGlobal(patch: Partial<GlobalConfig>) {
    setDraft((prev) => (prev ? { ...prev, global: { ...prev.global, ...patch } } : prev));
  }

  // Tunable edits mirror to the same-level sibling of the other size when linkSizes is on — for
  // every key EXCEPT nodes (per-size: 50k 6x6 vs 40k 5x5). searchMode and depth are per-level.
  function updateBotTunables(name: string, patch: Partial<BotTunables>) {
    setDraft((prev) => {
      if (!prev) return prev;
      const target = prev.bots.find((b) => b.name === name);
      if (!target) return prev;
      const mirrorPatch: Partial<BotTunables> = {};
      for (const [k, v] of Object.entries(patch)) {
        if (k !== "nodes") (mirrorPatch as Record<string, unknown>)[k] = v;
      }
      const siblingName =
        prev.global.linkSizes && Object.keys(mirrorPatch).length
          ? prev.bots.find((b) => b.level === target.level && b.name !== name)?.name
          : undefined;
      return {
        ...prev,
        bots: prev.bots.map((b) => {
          if (b.name === name) return { ...b, tunables: { ...b.tunables, ...patch } };
          if (b.name === siblingName) return { ...b, tunables: { ...b.tunables, ...mirrorPatch } };
          return b;
        }),
      };
    });
  }

  function updateBotAdvanced(name: string, patch: Partial<BotAdvanced>) {
    setDraft((prev) =>
      prev
        ? {
            ...prev,
            bots: prev.bots.map((b) =>
              b.name === name ? { ...b, advanced: { ...b.advanced, ...patch } } : b
            ),
          }
        : prev
    );
  }

  function updateBotEnabled(name: string, enabled: boolean) {
    setDraft((prev) =>
      prev
        ? { ...prev, bots: prev.bots.map((b) => (b.name === name ? { ...b, enabled } : b)) }
        : prev
    );
  }

  function updateBotOfferUndos(name: string, offerUndos: boolean) {
    setDraft((prev) =>
      prev
        ? { ...prev, bots: prev.bots.map((b) => (b.name === name ? { ...b, offerUndos } : b)) }
        : prev
    );
  }

  function updateBotUndoOfferLimit(name: string, undoOfferLimit: number | null) {
    setDraft((prev) =>
      prev
        ? { ...prev, bots: prev.bots.map((b) => (b.name === name ? { ...b, undoOfferLimit } : b)) }
        : prev
    );
  }

  function revert() {
    if (baseline) setDraft(structuredClone(baseline));
  }

  async function save() {
    if (!draft) return;
    setSaving(true);
    try {
      const saved = await api.putConfig(draft);
      setBaseline(saved);
      setDraft(structuredClone(saved));
      setConfigVersion((v) => v + 1);
      setToast({ msg: "Config saved", severity: "success" });
    } catch (e) {
      setToast({ msg: `Save failed — ${(e as Error).message}`, severity: "error" });
    } finally {
      setSaving(false);
    }
  }

  return (
    <Box sx={{ display: "flex", flexDirection: "column", minHeight: "100vh" }}>
      <AppBar position="static">
        <Toolbar>
          <Typography variant="h6" sx={{ flexGrow: 1 }}>
            PlayTak Bot Manager
          </Typography>
          <Tooltip title={meta.label}>
            <Box sx={{ display: "flex", alignItems: "center", gap: 0.5 }}>
              <CircleIcon sx={{ fontSize: 14, color: meta.color }} />
              <Typography variant="body2">{meta.label}</Typography>
            </Box>
          </Tooltip>
        </Toolbar>
      </AppBar>

      {/* Control bar. Start/Stop/Restart are wired in Phase 4/5; disabled while dirty so a launch
          always uses saved config. Save/Revert appear when there are unsaved edits. */}
      <Box sx={{ borderBottom: 1, borderColor: "divider", bgcolor: "background.paper" }}>
        <Container sx={{ py: 1.5 }}>
          <Box sx={{ display: "flex", flexWrap: "wrap", alignItems: "center", columnGap: 1.5, rowGap: 1 }}>
            {/* Primary controls: on small screens they take their own row as 3 equal columns; inline on sm+. */}
            <Box
              sx={{
                flexBasis: { xs: "100%", sm: "auto" },
                display: { xs: "grid", sm: "flex" },
                gridTemplateColumns: { xs: "repeat(3, minmax(0, 1fr))" },
                gap: 1.5,
                alignItems: "center",
              }}
            >
              <Tooltip title={dirty ? "Save changes before launching" : "Start all enabled bots"}>
                <Box component="span" sx={{ display: "block" }}>
                  <Button
                    variant="contained"
                    color="success"
                    startIcon={compact ? undefined : <PlayArrowIcon />}
                    disabled={dirty}
                    onClick={() => runAction(() => api.startAll(), "Start all")}
                    sx={{ width: { xs: "100%", sm: "auto" }, px: { xs: 1, sm: 2 }, textTransform: { xs: "none", sm: "uppercase" } }}
                  >
                    Start all
                  </Button>
                </Box>
              </Tooltip>
              <Button
                variant="outlined"
                color="error"
                startIcon={compact ? undefined : <StopIcon />}
                onClick={() => runAction(() => api.stopAll(), "Stop all")}
                sx={{ width: { xs: "100%", sm: "auto" }, px: { xs: 1, sm: 2 }, textTransform: { xs: "none", sm: "uppercase" } }}
              >
                Stop all
              </Button>
              <Tooltip title={dirty ? "Save changes before launching" : "Restart all enabled bots"}>
                <Box component="span" sx={{ display: "block" }}>
                  <Button
                    variant="outlined"
                    startIcon={compact ? undefined : <RestartAltIcon />}
                    disabled={dirty}
                    onClick={() => runAction(() => api.restartAll(), "Restart all")}
                    sx={{ width: { xs: "100%", sm: "auto" }, px: { xs: 1, sm: 2 }, textTransform: { xs: "none", sm: "uppercase" } }}
                  >
                    Restart all
                  </Button>
                </Box>
              </Tooltip>
            </Box>

            {/* Spacer pushes the save controls to the right on sm+ (hidden on xs so they wrap below). */}
            <Box sx={{ display: { xs: "none", sm: "block" }, flexGrow: 1 }} />

            {/* Save controls: hidden until there are pending changes on xs (then their own row); on sm+
                they stay visible (disabled when clean), matching the previous desktop layout. */}
            <Box
              sx={{
                flexBasis: { xs: "100%", sm: "auto" },
                display: dirty ? "flex" : { xs: "none", sm: "flex" },
                gap: 1.5,
                alignItems: "center",
                justifyContent: { sm: "flex-end" },
              }}
            >
              {dirty && (
                <Chip
                  color="warning"
                  size="small"
                  label="Unsaved changes"
                  sx={{ display: { xs: "none", sm: "inline-flex" } }}
                />
              )}
              <Button
                variant="text"
                startIcon={<UndoIcon />}
                disabled={!dirty || saving}
                onClick={revert}
                sx={{ flex: { xs: 1, sm: "none" } }}
              >
                Revert
              </Button>
              <Button
                variant="contained"
                startIcon={<SaveIcon />}
                disabled={!dirty || saving}
                onClick={save}
                sx={{ flex: { xs: 1, sm: "none" } }}
              >
                Save
              </Button>
            </Box>
          </Box>
        </Container>
      </Box>

      <Container sx={{ py: 3 }}>
        {loadError && (
          <Alert severity="error" sx={{ mb: 2 }}>
            Failed to load config: {loadError}
          </Alert>
        )}

        {orphans.length > 0 && (
          <Alert
            severity="warning"
            sx={{ mb: 2 }}
            action={
              <Button
                color="inherit"
                size="small"
                onClick={() => runAction(() => api.stopOrphans(), "Stop external bots")}
              >
                Stop them
              </Button>
            }
          >
            {orphans.length} bot{orphans.length > 1 ? "s" : ""} running outside this panel (e.g. from
            run-ladder.ps1): {orphans.map((o) => o.user || `pid ${o.pid}`).join(", ")}. This panel
            can't control them — stop them to manage from here.
          </Alert>
        )}

        {hashTotalMb > RAM_WARN_MB && (
          <Alert severity="warning" sx={{ mb: 2 }}>
            Enabled bots reserve ~{hashTotalMb} MB of hash tables (plus search memory). That's high —
            lower Hash (MB) in Advanced or disable some bots if the machine is tight on RAM.
          </Alert>
        )}

        {!draft || !schema ? (
          !loadError && (
            <Box sx={{ display: "flex", justifyContent: "center", py: 6 }}>
              <CircularProgress />
            </Box>
          )
        ) : (
          <>
            <GlobalPanel config={draft} schema={schema} onGlobal={updateGlobal} />
            {LEVEL_ORDER.map((level) => {
              const bots = draft.bots
                .filter((b) => b.level === level)
                .sort((a, b) => b.size - a.size); // 6x6 then 5x5
              return (
                <Box key={level} sx={{ mb: 2 }}>
                  <Box
                    sx={{
                      display: "grid",
                      gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" },
                      gap: 2,
                    }}
                  >
                    {bots.map((bot) => (
                      <BotCard
                        key={bot.name}
                        bot={bot}
                        schema={schema}
                        status={statuses[bot.name]}
                        gameStartedAt={
                          statuses[bot.name]?.inGame && statuses[bot.name]?.gameId
                            ? gameStartTimes[`${bot.name}:${statuses[bot.name]!.gameId}`]
                            : undefined
                        }
                        dirty={dirty}
                        onTunables={(p) => updateBotTunables(bot.name, p)}
                        onAdvanced={(p) => updateBotAdvanced(bot.name, p)}
                        onEnabled={(v) => updateBotEnabled(bot.name, v)}
                        onOfferUndos={(v) => updateBotOfferUndos(bot.name, v)}
                        onUndoOfferLimit={(v) => updateBotUndoOfferLimit(bot.name, v)}
                        onStart={() => runAction(() => api.startBot(bot.name), `Start ${bot.name}`)}
                        onStop={() => runAction(() => api.stopBot(bot.name), `Stop ${bot.name}`)}
                        onRestart={() =>
                          runAction(
                            () => api.restartBot(bot.name),
                            statuses[bot.name]?.inGame
                              ? `${bot.name} will restart after its game`
                              : `Restart ${bot.name}`
                          )
                        }
                        onLogs={() => setLogsTarget({ name: bot.name })}
                      />
                    ))}
                  </Box>
                </Box>
              );
            })}
            <HistoryCard
              onOpenGame={(name, gameId) => setLogsTarget({ name, tab: "history", gameId })}
            />
          </>
        )}
      </Container>

      <LogsDrawer
        target={logsTarget}
        onClose={() => setLogsTarget(null)}
        configVersion={configVersion}
      />

      <Snackbar
        open={!!toast}
        autoHideDuration={4000}
        onClose={() => setToast(null)}
        anchorOrigin={{ vertical: "bottom", horizontal: "center" }}
      >
        {toast ? (
          <Alert severity={toast.severity} onClose={() => setToast(null)} variant="filled">
            {toast.msg}
          </Alert>
        ) : undefined}
      </Snackbar>
    </Box>
  );
}
