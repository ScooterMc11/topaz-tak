// Slide-in drawer for a bot's logs, with two tabs:
//  • Live    — the actively-polling stdout/stderr stream (as before).
//  • History — the last 20 saved games (left: list w/ opponent + result + date; right: that game's log).
import { useEffect, useRef, useState } from "react";
import Drawer from "@mui/material/Drawer";
import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import IconButton from "@mui/material/IconButton";
import Tabs from "@mui/material/Tabs";
import Tab from "@mui/material/Tab";
import List from "@mui/material/List";
import ListItemButton from "@mui/material/ListItemButton";
import Chip from "@mui/material/Chip";
import CircularProgress from "@mui/material/CircularProgress";
import CloseIcon from "@mui/icons-material/Close";
import { api } from "../api";
import type { GameSummary } from "../types";
import { fmtDateTime, fmtDuration, OUTCOME_COLOR } from "../format";

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
}

const MONO_SX = {
  fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
  fontSize: 12,
  whiteSpace: "pre-wrap" as const,
  wordBreak: "break-word" as const,
};

export default function LogsDrawer({ target, onClose }: LogsDrawerProps) {
  const botName = target?.name ?? null;
  const [tab, setTab] = useState<"live" | "history">("live");
  const [liveLines, setLiveLines] = useState<string[]>([]);
  const [games, setGames] = useState<GameSummary[] | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [gameLines, setGameLines] = useState<string[] | null>(null);
  const liveBottomRef = useRef<HTMLDivElement>(null);

  // Reset everything when the drawer opens on a different target. Honor a requested tab / game so the
  // combined History card can open a bot straight to a specific saved game.
  useEffect(() => {
    setTab(target?.tab ?? "live");
    setLiveLines([]);
    setGames(null);
    setSelectedId(target?.gameId ?? null);
    setGameLines(null);
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

  // Load the saved-games list when the History tab is opened.
  useEffect(() => {
    if (!botName || tab !== "history") return;
    let cancelled = false;
    api
      .getGames(botName)
      .then(({ games }) => !cancelled && setGames(games))
      .catch(() => !cancelled && setGames([]));
    return () => {
      cancelled = true;
    };
  }, [botName, tab]);

  // Load a selected game's log.
  useEffect(() => {
    if (!botName || !selectedId) {
      setGameLines(null);
      return;
    }
    let cancelled = false;
    setGameLines(null);
    api
      .getGameLog(botName, selectedId)
      .then(({ lines }) => !cancelled && setGameLines(lines))
      .catch(() => !cancelled && setGameLines([]));
    return () => {
      cancelled = true;
    };
  }, [botName, selectedId]);

  return (
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
        <Tabs value={tab} onChange={(_e, v) => setTab(v)} sx={{ borderBottom: 1, borderColor: "divider", px: 1.5, flexShrink: 0 }}>
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
        ) : (
          <Box sx={{ flexGrow: 1, display: "flex", minHeight: 0 }}>
            {/* Left: game list */}
            <Box sx={{ width: 260, flexShrink: 0, overflow: "auto", borderRight: 1, borderColor: "divider" }}>
              {games === null ? (
                <Box sx={{ display: "flex", justifyContent: "center", py: 3 }}>
                  <CircularProgress size={22} />
                </Box>
              ) : games.length === 0 ? (
                <Typography variant="body2" color="text.secondary" sx={{ p: 2 }}>
                  No saved games yet. Finished games appear here (up to 20).
                </Typography>
              ) : (
                <List dense disablePadding>
                  {games.map((g) => (
                    <ListItemButton
                      key={g.id}
                      selected={g.id === selectedId}
                      onClick={() => setSelectedId(g.id)}
                      sx={{ display: "block", py: 1, borderBottom: 1, borderColor: "divider" }}
                    >
                      <Box sx={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 1 }}>
                        <Typography variant="body2" noWrap sx={{ fontWeight: 600 }}>
                          vs {g.opponent ?? "unknown"}
                        </Typography>
                        <Chip
                          size="small"
                          color={OUTCOME_COLOR[g.outcome]}
                          label={g.outcome === "unknown" ? g.result || "?" : g.outcome}
                          sx={{ height: 18, fontSize: 10, textTransform: "capitalize" }}
                        />
                      </Box>
                      <Typography variant="caption" color="text.secondary">
                        {fmtDateTime(g.startedAt)} · {fmtDuration(g.startedAt, g.endedAt)} · {g.moveCount} moves
                      </Typography>
                    </ListItemButton>
                  ))}
                </List>
              )}
            </Box>
            {/* Right: selected game's log */}
            <Box sx={{ flexGrow: 1, overflow: "auto", p: 1.5, ...MONO_SX }}>
              {!selectedId ? (
                <Typography variant="body2" color="text.secondary">
                  Select a game on the left to view its log.
                </Typography>
              ) : gameLines === null ? (
                <Box sx={{ display: "flex", justifyContent: "center", py: 3 }}>
                  <CircularProgress size={22} />
                </Box>
              ) : (
                gameLines.map((l, i) => <div key={i}>{l}</div>)
              )}
            </Box>
          </Box>
        )}
      </Box>
    </Drawer>
  );
}
