// Combined game history across all bots. Full-width, collapsible (expanded by default) card at the
// bottom of the page. Merges every bot's saved games (the existing per-bot /games endpoint) into one
// newest-first list. Clicking a row opens that bot's Logs drawer on the History tab with the game open.
//   • Desktop (sm+): a real table.
//   • Phone (xs):    stacked cards, no horizontal scrolling.
import { useEffect, useMemo, useState } from "react";
import Card from "@mui/material/Card";
import CardContent from "@mui/material/CardContent";
import Collapse from "@mui/material/Collapse";
import Typography from "@mui/material/Typography";
import Box from "@mui/material/Box";
import IconButton from "@mui/material/IconButton";
import Tooltip from "@mui/material/Tooltip";
import Chip from "@mui/material/Chip";
import CircularProgress from "@mui/material/CircularProgress";
import List from "@mui/material/List";
import ListItemButton from "@mui/material/ListItemButton";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import useMediaQuery from "@mui/material/useMediaQuery";
import { useTheme } from "@mui/material/styles";
import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import ChevronRightIcon from "@mui/icons-material/ChevronRight";
import { api } from "../api";
import type { GameSummary } from "../types";
import { fmtDateTime, fmtDuration, OUTCOME_COLOR } from "../format";

// A game plus which bot played it (the combined list's extra dimension).
type HistoryRow = GameSummary & { botName: string };

const MAX_ROWS = 50;
const REFRESH_MS = 15000;

interface HistoryCardProps {
  botNames: string[];
  onOpenGame: (botName: string, gameId: string) => void;
}

function OutcomeChip({ g }: { g: GameSummary }) {
  return (
    <Chip
      size="small"
      color={OUTCOME_COLOR[g.outcome]}
      label={g.outcome === "unknown" ? g.result || "?" : g.outcome}
      sx={{ height: 18, fontSize: 10, textTransform: "capitalize" }}
    />
  );
}

export default function HistoryCard({ botNames, onOpenGame }: HistoryCardProps) {
  const theme = useTheme();
  const stacked = useMediaQuery(theme.breakpoints.down("sm")); // phones: cards instead of a table
  const [collapsed, setCollapsed] = useState(false); // expanded by default
  const [rows, setRows] = useState<HistoryRow[] | null>(null);

  // Stable key so the fetch effect doesn't restart on every parent render (botNames is rebuilt each time).
  const namesKey = botNames.join(",");

  // Fetch every bot's games in parallel, merge newest-first, cap the list. Poll while expanded so a
  // freshly-finished game shows up; skip the work entirely while collapsed.
  useEffect(() => {
    if (collapsed) return;
    const names = namesKey ? namesKey.split(",") : [];
    let cancelled = false;
    async function load() {
      const results = await Promise.all(
        names.map(async (name) => {
          try {
            const { games } = await api.getGames(name);
            return games.map((g) => ({ ...g, botName: name }));
          } catch {
            return [] as HistoryRow[];
          }
        })
      );
      if (cancelled) return;
      const merged = results
        .flat()
        .sort((a, b) => b.startedAt - a.startedAt)
        .slice(0, MAX_ROWS);
      setRows(merged);
    }
    load();
    const id = setInterval(load, REFRESH_MS);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [collapsed, namesKey]);

  const body = useMemo(() => {
    if (rows === null) {
      return (
        <Box sx={{ display: "flex", justifyContent: "center", py: 4 }}>
          <CircularProgress size={24} />
        </Box>
      );
    }
    if (rows.length === 0) {
      return (
        <Typography variant="body2" color="text.secondary" sx={{ py: 1 }}>
          No games played yet. Finished games from all bots show up here.
        </Typography>
      );
    }

    if (stacked) {
      return (
        <List dense disablePadding sx={{ border: 1, borderColor: "divider", borderRadius: 1 }}>
          {rows.map((g) => (
            <ListItemButton
              key={`${g.botName}:${g.id}`}
              onClick={() => onOpenGame(g.botName, g.id)}
              sx={{ display: "block", py: 1, borderBottom: 1, borderColor: "divider", "&:last-child": { borderBottom: 0 } }}
            >
              <Box sx={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 1 }}>
                <Typography variant="body2" noWrap sx={{ fontWeight: 600 }}>
                  {g.botName} <Box component="span" sx={{ fontWeight: 400, color: "text.secondary" }}>vs {g.opponent ?? "unknown"}</Box>
                </Typography>
                <OutcomeChip g={g} />
              </Box>
              <Typography variant="caption" color="text.secondary">
                {fmtDateTime(g.startedAt)} · {fmtDuration(g.startedAt, g.endedAt)} · {g.moveCount} moves
              </Typography>
            </ListItemButton>
          ))}
        </List>
      );
    }

    return (
      <TableContainer sx={{ border: 1, borderColor: "divider", borderRadius: 1 }}>
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>Bot</TableCell>
              <TableCell>Opponent</TableCell>
              <TableCell>Result</TableCell>
              <TableCell>Date</TableCell>
              <TableCell align="right">Duration</TableCell>
              <TableCell align="right">Moves</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {rows.map((g) => (
              <TableRow
                key={`${g.botName}:${g.id}`}
                hover
                onClick={() => onOpenGame(g.botName, g.id)}
                sx={{ cursor: "pointer", "&:last-child td": { border: 0 } }}
              >
                <TableCell sx={{ fontWeight: 600 }}>{g.botName}</TableCell>
                <TableCell>vs {g.opponent ?? "unknown"}</TableCell>
                <TableCell><OutcomeChip g={g} /></TableCell>
                <TableCell sx={{ whiteSpace: "nowrap" }}>{fmtDateTime(g.startedAt)}</TableCell>
                <TableCell align="right" sx={{ whiteSpace: "nowrap" }}>{fmtDuration(g.startedAt, g.endedAt)}</TableCell>
                <TableCell align="right">{g.moveCount}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </TableContainer>
    );
  }, [rows, stacked, onOpenGame]);

  return (
    <Card variant="outlined" sx={{ mb: 3 }}>
      <CardContent sx={{ "&:last-child": { pb: collapsed ? 2 : undefined } }}>
        <Box sx={{ display: "flex", alignItems: "center", gap: 0.5, mb: collapsed ? 0 : 2 }}>
          <Tooltip title={collapsed ? "Expand" : "Collapse"}>
            <IconButton size="small" onClick={() => setCollapsed((c) => !c)}>
              {collapsed ? <ChevronRightIcon fontSize="small" /> : <ExpandMoreIcon fontSize="small" />}
            </IconButton>
          </Tooltip>
          <Typography variant="subtitle1">History</Typography>
          {rows && rows.length > 0 && (
            <Typography variant="caption" color="text.secondary" sx={{ ml: 0.5 }}>
              latest {rows.length} across all bots
            </Typography>
          )}
        </Box>

        <Collapse in={!collapsed} timeout="auto" unmountOnExit>
          {body}
        </Collapse>
      </CardContent>
    </Card>
  );
}
