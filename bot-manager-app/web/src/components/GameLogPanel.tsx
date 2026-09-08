// One saved game: its captured log above a sticky footer showing the configuration the bot was
// actually running for that game plus a PTN Ninja review link. Shared by the Logs drawer's
// side-by-side layout (desktop) and the full-screen dialog (phones), so the two can't drift.
import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import Chip from "@mui/material/Chip";
import Button from "@mui/material/Button";
import Tooltip from "@mui/material/Tooltip";
import CircularProgress from "@mui/material/CircularProgress";
import OpenInNewIcon from "@mui/icons-material/OpenInNew";
import type { GameDetail } from "../types";
import { configChips, fmtDateTime, fmtDuration, fmtResult } from "../format";

export const MONO_SX = {
  fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
  fontSize: 12,
  whiteSpace: "pre-wrap" as const,
  wordBreak: "break-word" as const,
};

interface GameLogPanelProps {
  game: GameDetail | null; // null while loading, or when nothing is selected
  emptyText?: string; // shown when no game is selected (desktop side-by-side layout)
  selected: boolean; // a game IS selected — null `game` then means "still loading"
}

export default function GameLogPanel({ game, emptyText, selected }: GameLogPanelProps) {
  if (!selected) {
    return (
      <Box sx={{ flexGrow: 1, overflow: "auto", p: 1.5 }}>
        <Typography variant="body2" color="text.secondary">
          {emptyText ?? "Select a game to view its log."}
        </Typography>
      </Box>
    );
  }

  if (!game) {
    return (
      <Box sx={{ flexGrow: 1, display: "flex", justifyContent: "center", py: 3 }}>
        <CircularProgress size={22} />
      </Box>
    );
  }

  const chips = game.config ? configChips(game.config) : null;

  return (
    <Box sx={{ flexGrow: 1, display: "flex", flexDirection: "column", minHeight: 0 }}>
      <Box sx={{ flexGrow: 1, overflow: "auto", p: 1.5, ...MONO_SX }}>
        {game.lines.length === 0 ? (
          <Typography variant="body2" color="text.secondary">
            This game's log wasn't captured.
          </Typography>
        ) : (
          game.lines.map((l, i) => <div key={i}>{l}</div>)
        )}
      </Box>

      {/* Sticky footer — the config this game was played under, and the review link. */}
      <Box
        sx={{
          flexShrink: 0,
          borderTop: 1,
          borderColor: "divider",
          bgcolor: "background.paper",
          px: 1.5,
          py: 1,
        }}
      >
        <Box
          sx={{
            display: "flex",
            alignItems: "flex-start",
            justifyContent: "space-between",
            gap: 1,
            flexWrap: { xs: "wrap", sm: "nowrap" },
          }}
        >
          <Box sx={{ minWidth: 0, flexGrow: 1 }}>
            <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 0.5 }}>
              vs {game.opponent ?? "unknown"}
              {game.opponentRating != null ? ` (${game.opponentRating})` : ""} · as{" "}
              {game.color ?? "?"} · {fmtResult(game.result)} · {fmtDateTime(game.startedAt)} ·{" "}
              {fmtDuration(game.startedAt, game.endedAt)}
              {game.undosAccepted > 0
                ? ` · ${game.undosAccepted} courtesy undo${game.undosAccepted === 1 ? "" : "s"} taken`
                : ""}
            </Typography>
            {chips ? (
              <Box sx={{ display: "flex", flexWrap: "wrap", gap: 0.5 }}>
                {chips.map((c) => (
                  <Chip
                    key={c.label}
                    size="small"
                    variant="outlined"
                    label={
                      <>
                        <Box component="span" sx={{ color: "text.secondary" }}>
                          {c.label}
                        </Box>{" "}
                        <Box component="span" sx={{ fontWeight: 600 }}>
                          {c.value}
                        </Box>
                      </>
                    }
                    sx={{ height: 20, fontSize: 11 }}
                  />
                ))}
              </Box>
            ) : (
              <Typography variant="caption" color="text.secondary">
                Configuration wasn't recorded for this game (imported from the pre-database logs).
              </Typography>
            )}
          </Box>

          <Tooltip
            title={
              game.reviewUrl
                ? "Open this game in PTN Ninja"
                : game.playtakGameId
                  ? "This game was played on a server without a public game-history page"
                  : "No PlayTak game id was recorded for this game"
            }
          >
            {/* span keeps the tooltip alive while the button is disabled. The link and disabled
                variants are separate elements because MUI's Button only accepts target/rel on its
                anchor overload. */}
            <Box component="span" sx={{ flexShrink: 0 }}>
              {game.reviewUrl ? (
                <Button
                  size="small"
                  variant="outlined"
                  endIcon={<OpenInNewIcon />}
                  href={game.reviewUrl}
                  target="_blank"
                  rel="noreferrer"
                >
                  PTN Ninja
                </Button>
              ) : (
                <Button size="small" variant="outlined" endIcon={<OpenInNewIcon />} disabled>
                  PTN Ninja
                </Button>
              )}
            </Box>
          </Tooltip>
        </Box>
      </Box>
    </Box>
  );
}
