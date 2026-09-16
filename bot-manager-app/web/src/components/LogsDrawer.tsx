// Slide-in drawer for a bot's logs, with two tabs:
//  • Live    — the actively-polling stdout/stderr stream.
//  • History — saved games from the database (left: list w/ opponent + result + date; right: that
//    game's log and a sticky config/review footer).
// The header carries two box plots of the opponent ELOs this bot has won and lost against, matched
// to its CURRENT configuration (see the /elo-stats route).
// On phones there isn't room for list + log side by side, so the list fills the panel and tapping a
// game opens its log in a full-screen dialog instead.
import { useEffect, useRef, useState } from "react";
import Drawer from "@mui/material/Drawer";
import Dialog from "@mui/material/Dialog";
import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import IconButton from "@mui/material/IconButton";
import Tabs from "@mui/material/Tabs";
import Tab from "@mui/material/Tab";
import List from "@mui/material/List";
import ListItemButton from "@mui/material/ListItemButton";
import Chip from "@mui/material/Chip";
import CircularProgress from "@mui/material/CircularProgress";
import useMediaQuery from "@mui/material/useMediaQuery";
import { useTheme } from "@mui/material/styles";
import CloseIcon from "@mui/icons-material/Close";
import { api } from "../api";
import type { EloStats, GameDetail, GameSummary } from "../types";
import { fmtDateTime, fmtDuration, fmtResult, OUTCOME_COLOR } from "../format";
import EloBoxPlots from "./EloBoxPlots";
import GameLogPanel, { MONO_SX } from "./GameLogPanel";

// What to open: a bot's logs, optionally jumping straight to a saved game in the History tab
// (used by the combined History card). null = drawer closed.
export interface LogsTarget {
  name: string;
  tab?: "live" | "history";
  gameId?: string;
}

interface LogsDrawerProps {
  target: LogsTarget | null;
  onClose: () => void;
  // Bumped by the parent whenever the config is saved, so the ELO plots re-match immediately
  // against the new configuration instead of waiting for the drawer to be reopened.
  configVersion?: number;
}

// How many saved games the list shows. The database keeps every game ever played.
const HISTORY_LIMIT = 30;

export default function LogsDrawer({ target, onClose, configVersion = 0 }: LogsDrawerProps) {
  const botName = target?.name ?? null;
  const theme = useTheme();
  const stacked = useMediaQuery(theme.breakpoints.down("sm")); // phones: list only + dialog
  const [tab, setTab] = useState<"live" | "history">("live");
  const [liveLines, setLiveLines] = useState<string[]>([]);
  const [games, setGames] = useState<GameSummary[] | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [game, setGame] = useState<GameDetail | null>(null);
  const [elo, setElo] = useState<EloStats | null>(null);
  const liveBottomRef = useRef<HTMLDivElement>(null);

  // Reset everything when the drawer opens on a different target. Honor a requested tab / game so the
  // combined History card can open a bot straight to a specific saved game.
  useEffect(() => {
    setTab(target?.tab ?? "live");
    setLiveLines([]);
    setGames(null);
    setSelectedId(target?.gameId ?? null);
    setGame(null);
  }, [target?.name, target?.tab, target?.gameId]);

  // Live tail — poll only while the Live tab is showing.
  useEffect(() => {
    if (!botName || tab !== "live") return;
    let cancelled = false;
    async function pull() {
      try {
        const { lines } = await api.getLogs(botName!, 400);
        if (!cancelled) setLiveLines(lines);
      } catch {
        /* transient — keep last lines */
      }
    }
    pull();
    const id = setInterval(pull, 2000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [botName, tab]);

  useEffect(() => {
    if (tab === "live") liveBottomRef.current?.scrollIntoView({ block: "end" });
  }, [liveLines, tab]);

  // ELO distributions for the current config. Kept in the header (both tabs), and refreshed on a
  // slow timer so ratings arriving from the background worker show up without a reload.
  useEffect(() => {
    if (!botName) return;
    let cancelled = false;
    setElo(null);
    const load = () =>
      api
        .getEloStats(botName)
        .then((s) => !cancelled && setElo(s))
        .catch(() => !cancelled && setElo(null));
    load();
    const id = setInterval(load, 20000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [botName, configVersion]);

  // Load the saved-games list when the History tab is opened.
  useEffect(() => {
    if (!botName || tab !== "history") return;
    let cancelled = false;
    api
      .getGames(botName, HISTORY_LIMIT)
      .then(({ games }) => !cancelled && setGames(games))
      .catch(() => !cancelled && setGames([]));
    return () => {
      cancelled = true;
    };
  }, [botName, tab]);

  // Load a selected game (log + the config it was played under).
  useEffect(() => {
    if (!botName || !selectedId) {
      setGame(null);
      return;
    }
    let cancelled = false;
    setGame(null);
    api
      .getGame(botName, selectedId)
      .then((g) => !cancelled && setGame(g))
      .catch(() => !cancelled && setGame(null));
    return () => {
      cancelled = true;
    };
  }, [botName, selectedId]);

  const gameList =
    games === null ? (
      <Box sx={{ display: "flex", justifyContent: "center", py: 3 }}>
        <CircularProgress size={22} />
      </Box>
    ) : games.length === 0 ? (
      <Typography variant="body2" color="text.secondary" sx={{ p: 2 }}>
        No saved games yet. Finished games appear here (newest {HISTORY_LIMIT}).
      </Typography>
    ) : (
      <List dense disablePadding>
        {games.map((g) => (
          <ListItemButton
            key={g.id}
            selected={!stacked && g.id === selectedId}
            onClick={() => setSelectedId(g.id)}
            sx={{ display: "block", py: 1, borderBottom: 1, borderColor: "divider" }}
          >
            <Box sx={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 1 }}>
              <Typography variant="body2" noWrap sx={{ fontWeight: 600 }}>
                vs {g.opponent ?? "unknown"}
                {g.opponentRating != null && (
                  <Box component="span" sx={{ fontWeight: 400, color: "text.secondary" }}>
                    {" "}
                    ({g.opponentRating})
                  </Box>
                )}
              </Typography>
              <Box sx={{ display: "flex", alignItems: "center", gap: 0.5, flexShrink: 0 }}>
                <Typography
                  variant="caption"
                  sx={{ fontFamily: "ui-monospace, monospace", color: "text.secondary" }}
                >
                  {fmtResult(g.result)}
                </Typography>
                <Chip
                  size="small"
                  color={OUTCOME_COLOR[g.outcome]}
                  label={g.outcome === "unknown" ? "?" : g.outcome}
                  sx={{ height: 18, fontSize: 10, textTransform: "capitalize" }}
                />
              </Box>
            </Box>
            <Typography variant="caption" color="text.secondary">
              {fmtDateTime(g.startedAt)} · {fmtDuration(g.startedAt, g.endedAt)} · {g.moveCount} moves
            </Typography>
          </ListItemButton>
        ))}
      </List>
    );

  return (
    <>
      <Drawer
        anchor="right"
        open={!!botName}
        onClose={onClose}
        // Cap to the visible viewport (dvh handles the mobile URL bar) so the panel never exceeds the
        // screen — the title + tabs stay pinned and only the log body scrolls.
        PaperProps={{ sx: { height: { xs: "100dvh", sm: "100%" }, maxHeight: "100dvh" } }}
      >
        <Box
          sx={{
            width: { xs: "100vw", sm: 640, md: 840 },
            display: "flex",
            flexDirection: "column",
            height: "100%",
            minHeight: 0,
          }}
        >
          <Box
            sx={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              px: 1.5,
              pt: 1.5,
              flexShrink: 0,
            }}
          >
            <Typography variant="subtitle1">{botName} · logs</Typography>
            <IconButton size="small" onClick={onClose}>
              <CloseIcon fontSize="small" />
            </IconButton>
          </Box>

          {/* Header stat: opponent ELOs won vs lost against, on the current configuration. */}
          <Box sx={{ px: 1.5, pt: 1, pb: 1.5, flexShrink: 0 }}>
            <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 0.5 }}>
              Opponent ELO on this configuration
            </Typography>
            {elo === null ? (
              <Box sx={{ display: "flex", justifyContent: "center", py: 1 }}>
                <CircularProgress size={18} />
              </Box>
            ) : (
              <EloBoxPlots
                won={elo.won}
                lost={elo.lost}
                matchedGames={elo.matchedGames}
                pendingRatings={elo.pendingRatings}
                walkovers={elo.walkovers}
                boardSize={elo.config.size}
              />
            )}
          </Box>

          <Tabs
            value={tab}
            onChange={(_e, v) => setTab(v)}
            sx={{ borderBottom: 1, borderColor: "divider", px: 1.5, flexShrink: 0 }}
          >
            <Tab value="live" label="Live" />
            <Tab value="history" label="History" />
          </Tabs>

          {tab === "live" ? (
            <Box sx={{ flexGrow: 1, overflow: "auto", p: 1.5, ...MONO_SX }}>
              {liveLines.length === 0 ? (
                <Typography variant="body2" color="text.secondary">
                  No output captured (bot not started by this panel, or just launched).
                </Typography>
              ) : (
                liveLines.map((l, i) => <div key={i}>{l}</div>)
              )}
              <div ref={liveBottomRef} />
            </Box>
          ) : stacked ? (
            /* Phones: the list owns the panel; the log opens in the dialog below. */
            <Box sx={{ flexGrow: 1, overflow: "auto", minHeight: 0 }}>{gameList}</Box>
          ) : (
            <Box sx={{ flexGrow: 1, display: "flex", minHeight: 0 }}>
              <Box sx={{ width: 260, flexShrink: 0, overflow: "auto", borderRight: 1, borderColor: "divider" }}>
                {gameList}
              </Box>
              <GameLogPanel
                game={game}
                selected={!!selectedId}
                emptyText="Select a game on the left to view its log."
              />
            </Box>
          )}
        </Box>
      </Drawer>

      {/* Phone-only: the selected game's log + sticky footer, full screen. */}
      <Dialog
        fullScreen
        open={stacked && !!botName && tab === "history" && !!selectedId}
        onClose={() => setSelectedId(null)}
      >
        <Box sx={{ display: "flex", flexDirection: "column", height: "100dvh", minHeight: 0 }}>
          <Box
            sx={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              px: 1.5,
              py: 1,
              borderBottom: 1,
              borderColor: "divider",
              flexShrink: 0,
            }}
          >
            <Typography variant="subtitle2" noWrap>
              {botName} · vs {game?.opponent ?? "…"}
            </Typography>
            <IconButton size="small" onClick={() => setSelectedId(null)}>
              <CloseIcon fontSize="small" />
            </IconButton>
          </Box>
          <GameLogPanel game={game} selected={!!selectedId} />
        </Box>
      </Dialog>
    </>
  );
}
