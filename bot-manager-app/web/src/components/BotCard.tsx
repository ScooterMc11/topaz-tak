// One bot's config card (editable). Sliders write through onTunables/onAdvanced/onEnabled; the
// 3-thumb gap slider uses disableSwap so thumbs can't cross (GapMin <= GapStd <= GapMax holds
// structurally). Advanced fields each have an override toggle (off = null = use script default).
import { useState, type ReactElement } from "react";
import Card from "@mui/material/Card";
import CardContent from "@mui/material/CardContent";
import CardHeader from "@mui/material/CardHeader";
import Collapse from "@mui/material/Collapse";
import Chip from "@mui/material/Chip";
import Box from "@mui/material/Box";
import Switch from "@mui/material/Switch";
import FormControlLabel from "@mui/material/FormControlLabel";
import Accordion from "@mui/material/Accordion";
import AccordionSummary from "@mui/material/AccordionSummary";
import AccordionDetails from "@mui/material/AccordionDetails";
import Typography from "@mui/material/Typography";
import TextField from "@mui/material/TextField";
import MenuItem from "@mui/material/MenuItem";
import Menu from "@mui/material/Menu";
import ListItemIcon from "@mui/material/ListItemIcon";
import ListItemText from "@mui/material/ListItemText";
import IconButton from "@mui/material/IconButton";
import Tooltip from "@mui/material/Tooltip";
import ToggleButton from "@mui/material/ToggleButton";
import ToggleButtonGroup from "@mui/material/ToggleButtonGroup";
import useMediaQuery from "@mui/material/useMediaQuery";
import { useTheme } from "@mui/material/styles";
import CircleIcon from "@mui/icons-material/Circle";
import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import ChevronRightIcon from "@mui/icons-material/ChevronRight";
import MoreVertIcon from "@mui/icons-material/MoreVert";
import PlayArrowIcon from "@mui/icons-material/PlayArrow";
import StopIcon from "@mui/icons-material/Stop";
import RestartAltIcon from "@mui/icons-material/RestartAlt";
import SubjectIcon from "@mui/icons-material/Subject";
import SliderRow from "./SliderRow";
import { UNDO_OFFERS_DEFAULT_LIMIT } from "../types";
import type { BotConfig, BotStatus, BotTunables, BotAdvanced, ConfigSchema, PriorityClass, SearchMode, NumRange } from "../types";

interface BotCardProps {
  bot: BotConfig;
  schema: ConfigSchema;
  status?: BotStatus;
  gameStartedAt?: number; // client-observed start time of the current game (for the duration chip)
  dirty: boolean;
  onTunables: (patch: Partial<BotTunables>) => void;
  onAdvanced: (patch: Partial<BotAdvanced>) => void;
  onEnabled: (enabled: boolean) => void;
  onOfferUndos: (offerUndos: boolean) => void;
  onUndoOfferLimit: (limit: number | null) => void;
  onStart: () => void;
  onStop: () => void;
  onRestart: () => void;
  onLogs: () => void;
}

function fmtUptime(sec: number): string {
  if (sec < 60) return `${sec}s`;
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  if (m < 60) return `${m}m ${s}s`;
  const h = Math.floor(m / 60);
  return `${h}h ${m % 60}m`;
}

// Map a bot state to a dot color + tooltip. `running` here = "managed by this panel and up"
// (controls stop/restart apply); `orphanRunning` = up but started outside the panel.
function statusMeta(status?: BotStatus): {
  color: string;
  label: string;
  running: boolean;
  orphan: boolean;
} {
  const up = status?.uptimeSec != null ? ` · up ${fmtUptime(status.uptimeSec)}` : "";
  switch (status?.state) {
    case "connected":
      return { color: "#4caf50", label: `Connected (pid ${status.pid})${up}`, running: true, orphan: false };
    case "running":
      return { color: "#03a9f4", label: `Running, connecting… (pid ${status.pid})${up}`, running: true, orphan: false };
    case "orphan":
      return {
        color: "#ab47bc",
        label: `Running outside this panel (pid ${status.pid})${
          status.connected ? " · connected" : ""
        }`,
        running: false,
        orphan: true,
      };
    case "exited":
      return { color: "#ff9800", label: `Exited (code ${status.exitCode ?? "?"})`, running: false, orphan: false };
    default:
      return { color: "#9e9e9e", label: "Not running", running: false, orphan: false };
  }
}

export default function BotCard({
  bot,
  schema,
  status,
  gameStartedAt,
  dirty,
  onTunables,
  onAdvanced,
  onEnabled,
  onOfferUndos,
  onUndoOfferLimit,
  onStart,
  onStop,
  onRestart,
  onLogs,
}: BotCardProps) {
  const t = bot.tunables;
  const r = schema.ranges;
  const defTime = schema.advancedDefaults.timeBySize[String(bot.size)];
  const st = statusMeta(status);
  const [collapsed, setCollapsed] = useState(true);
  const theme = useTheme();
  const compact = useMediaQuery(theme.breakpoints.down("sm")); // small screens => ellipsis menu
  const [actionMenuAnchor, setActionMenuAnchor] = useState<null | HTMLElement>(null);

  // Lifecycle actions, shared by the inline icon buttons (sm+) and the ellipsis menu (xs).
  const actions: {
    key: string;
    label: string;
    icon: ReactElement;
    onClick: () => void;
    disabled: boolean;
    color: "success" | "error" | "default";
    tip: string;
  }[] = [
    { key: "start", label: "Start", icon: <PlayArrowIcon fontSize="small" />, onClick: onStart, disabled: st.running || dirty, color: "success", tip: dirty ? "Save changes first" : "Start" },
    { key: "stop", label: "Stop", icon: <StopIcon fontSize="small" />, onClick: onStop, disabled: !st.running, color: "error", tip: "Stop" },
    { key: "restart", label: "Restart", icon: <RestartAltIcon fontSize="small" />, onClick: onRestart, disabled: dirty, color: "default", tip: dirty ? "Save changes first" : "Restart" },
    { key: "logs", label: "Logs", icon: <SubjectIcon fontSize="small" />, onClick: onLogs, disabled: false, color: "default", tip: "Logs" },
  ];

  // Status chips (in-game / restart-queued). Rendered in the action column on sm+, but on small
  // screens they'd widen the header and shove the ellipsis off the edge, so there they wrap to their
  // own row below the header instead (see below). `compact` guarantees only one placement renders.
  const hasChips = !!(status?.inGame || status?.restartQueued);
  const statusChips = (
    <>
      {status?.inGame && (
        <Chip
          size="small"
          color="success"
          label={
            (status.opponent ? `Playing vs ${status.opponent}` : "In game") +
            (gameStartedAt ? ` · ${fmtUptime(Math.floor((Date.now() - gameStartedAt) / 1000))}` : "")
          }
          sx={{ height: 20, fontSize: 11, fontWeight: 600, maxWidth: 240 }}
        />
      )}
      {status?.restartQueued && (
        <Chip
          size="small"
          color="warning"
          icon={<RestartAltIcon />}
          label="Will restart after this game"
          sx={{ height: 20, fontSize: 11, fontWeight: 600, maxWidth: 240 }}
        />
      )}
    </>
  );

  return (
    <Card variant="outlined" sx={{ height: "100%" }}>
      <CardHeader
        avatar={
          <Tooltip title={collapsed ? "Expand" : "Collapse"}>
            <IconButton size="small" onClick={() => setCollapsed((c) => !c)}>
              {collapsed ? <ChevronRightIcon fontSize="small" /> : <ExpandMoreIcon fontSize="small" />}
            </IconButton>
          </Tooltip>
        }
        title={bot.name}
        titleTypographyProps={{ variant: "subtitle1" }}
        subheader={
          <Box sx={{ display: "flex", alignItems: "center", gap: 0.5, minWidth: 0 }}>
            <CircleIcon sx={{ fontSize: 12, color: st.color, flexShrink: 0 }} />
            <Typography variant="caption" color="text.secondary" noWrap>
              {st.label}
            </Typography>
          </Box>
        }
        action={
          // Column: controls on top (right-aligned), status chips below them. On small screens the
          // four lifecycle buttons collapse into a single ellipsis menu to save header width.
          <Box sx={{ display: "flex", flexDirection: "column", alignItems: "flex-end", pr: 0.5, pt: 0.5, gap: 0.5 }}>
            {compact ? (
              <>
                <IconButton size="small" onClick={(e) => setActionMenuAnchor(e.currentTarget)}>
                  <MoreVertIcon fontSize="small" />
                </IconButton>
                <Menu
                  anchorEl={actionMenuAnchor}
                  open={!!actionMenuAnchor}
                  onClose={() => setActionMenuAnchor(null)}
                >
                  {actions.map((a) => (
                    <MenuItem
                      key={a.key}
                      disabled={a.disabled}
                      onClick={() => {
                        setActionMenuAnchor(null);
                        a.onClick();
                      }}
                    >
                      <ListItemIcon sx={{ color: a.color !== "default" ? `${a.color}.main` : undefined }}>
                        {a.icon}
                      </ListItemIcon>
                      <ListItemText>{a.label}</ListItemText>
                    </MenuItem>
                  ))}
                </Menu>
              </>
            ) : (
              <Box sx={{ display: "flex", alignItems: "center", flexShrink: 0 }}>
                {actions.map((a) => (
                  <Tooltip key={a.key} title={a.tip}>
                    <span>
                      <IconButton size="small" color={a.color} disabled={a.disabled} onClick={a.onClick}>
                        {a.icon}
                      </IconButton>
                    </span>
                  </Tooltip>
                ))}
              </Box>
            )}
            {/* On sm+ the chips sit under the buttons; on xs they move to their own row below (see next). */}
            {!compact && statusChips}
          </Box>
        }
      />
      {/* Small screens: chips wrap to their own full-width row so the ellipsis stays reachable. */}
      {compact && hasChips && (
        <Box sx={{ display: "flex", flexWrap: "wrap", gap: 0.5, px: 2, pb: 1 }}>{statusChips}</Box>
      )}
      <Collapse in={!collapsed} timeout="auto" unmountOnExit>
      <CardContent sx={{ pt: 0 }}>
        <Box sx={{ display: "flex", flexWrap: "wrap", alignItems: "center", columnGap: 2, mb: 1 }}>
          <FormControlLabel
            control={
              <Switch
                size="small"
                checked={bot.enabled}
                onChange={(e) => onEnabled(e.target.checked)}
              />
            }
            label="Enabled"
          />
          <Tooltip
            title={'When an opponent hands this bot an instant road win that they could have avoided, it offers them the undo and says so in chat, then plays the win if they say "gg" or go quiet for a minute. Does not affect the configuration the ELO plots group by.'}
          >
            <FormControlLabel
              control={
                <Switch
                  size="small"
                  checked={bot.offerUndos}
                  onChange={(e) => onOfferUndos(e.target.checked)}
                />
              }
              label="Offer undos"
            />
          </Tooltip>
        </Box>

        {/* Only meaningful while the switch is on. Off = unlimited, which is the default. */}
        {bot.offerUndos && (
          <Box sx={{ mb: 1.5 }}>
            <FormControlLabel
              control={
                <Switch
                  size="small"
                  checked={bot.undoOfferLimit != null}
                  onChange={(e) =>
                    onUndoOfferLimit(e.target.checked ? UNDO_OFFERS_DEFAULT_LIMIT : null)
                  }
                />
              }
              label={
                <Typography variant="body2" color="text.secondary">
                  Limit undos per game {bot.undoOfferLimit == null ? "— unlimited" : ""}
                </Typography>
              }
            />
            {bot.undoOfferLimit != null && (
              <SliderRow
                label="Undos per game"
                value={bot.undoOfferLimit}
                range={r.undoOfferLimit}
                helper="After this many accepted-or-declined offers, the bot just plays the win"
                onChange={(v) => onUndoOfferLimit(v as number)}
              />
            )}
          </Box>
        )}

        <Box sx={{ mb: 1.5 }}>
          <Box sx={{ display: "flex", justifyContent: "space-between", alignItems: "center", mb: 0.5 }}>
            <Typography variant="body2" color="text.secondary">
              Strength knob
            </Typography>
            <ToggleButtonGroup
              size="small"
              exclusive
              value={t.searchMode}
              onChange={(_e, v: SearchMode | null) => {
                if (v) onTunables({ searchMode: v });
              }}
            >
              <ToggleButton value="depth" sx={{ px: 1.5, py: 0.25 }}>
                Depth
              </ToggleButton>
              <ToggleButton value="nodes" sx={{ px: 1.5, py: 0.25 }}>
                Nodes
              </ToggleButton>
            </ToggleButtonGroup>
          </Box>
          {t.searchMode === "depth" ? (
            <SliderRow
              label="Search depth"
              value={t.depth}
              range={r.depth}
              helper="Fixed iterative-deepening depth, same regardless of board complexity"
              onChange={(v) => onTunables({ depth: v as number })}
            />
          ) : (
            <SliderRow
              label="Nodes"
              value={t.nodes}
              range={r.nodes}
              helper="Per-move search node cap (predictable time; varies strength with position)"
              onChange={(v) => onTunables({ nodes: v as number })}
            />
          )}
        </Box>
        <SliderRow
          label="Drop rate (Min / Std / Max)"
          value={[t.gapMin, t.gapStd, t.gapMax]}
          range={r.gap}
          disableSwap
          valueText={`${t.gapMin} / ${t.gapStd} / ${t.gapMax}`}
          helper="Min = losing (≤−50cp, strongest) · Std = even · Max = winning (≥+50cp, weakest)"
          onChange={(v) => {
            const [gapMin, gapStd, gapMax] = v as number[];
            onTunables({ gapMin, gapStd, gapMax });
          }}
        />
        <SliderRow
          label="Always allow ≥"
          value={t.alwaysAllow}
          range={r.alwaysAllow}
          valueText={`${t.alwaysAllow >= 0 ? "+" : ""}${t.alwaysAllow} cp`}
          helper="Moves scoring at least this stay eligible even outside the drop-rate band (never in a solved position)"
          onChange={(v) => onTunables({ alwaysAllow: v as number })}
        />
        <Box sx={{ display: "flex", gap: 2 }}>
          <Box sx={{ flex: 1, minWidth: 0 }}>
            <SliderRow
              label="Bias slope"
              value={t.biasSlope}
              range={r.biasSlope}
              helper={`worst:best odds = 1 + slope = ${(1 + t.biasSlope).toFixed(1)}; 0 = uniform`}
              onChange={(v) => onTunables({ biasSlope: v as number })}
            />
          </Box>
          <Box sx={{ flex: 1, minWidth: 0 }}>
            <SliderRow
              label="Move pool (MultiPV)"
              value={t.movePool}
              range={r.movePool}
              onChange={(v) => onTunables({ movePool: v as number })}
            />
          </Box>
        </Box>

        <Accordion disableGutters elevation={0} sx={{ mt: 1, bgcolor: "transparent" }}>
          <AccordionSummary expandIcon={<ExpandMoreIcon />} sx={{ px: 0 }}>
            <Typography variant="body2" color="text.secondary">
              Advanced
            </Typography>
          </AccordionSummary>
          <AccordionDetails sx={{ px: 0 }}>
            <AdvancedNum
              label="Abort depth"
              value={bot.advanced.abortDepth}
              def={schema.advancedDefaults.abortDepth}
              range={r.abortDepth}
              onChange={(v) => onAdvanced({ abortDepth: v })}
            />
            <AdvancedNum
              label="Time (s)"
              value={bot.advanced.time}
              def={defTime}
              range={r.time}
              onChange={(v) => onAdvanced({ time: v })}
            />
            <AdvancedNum
              label="Hash (MB)"
              value={bot.advanced.hashMb}
              def={schema.advancedDefaults.hashMb}
              range={r.hashMb}
              onChange={(v) => onAdvanced({ hashMb: v })}
            />
            <AdvancedPriority
              value={bot.advanced.priority}
              def={schema.advancedDefaults.priority}
              options={schema.priorities}
              onChange={(v) => onAdvanced({ priority: v })}
            />
          </AccordionDetails>
        </Accordion>
      </CardContent>
      </Collapse>
    </Card>
  );
}

// Numeric advanced override: a toggle to override the default, revealing a slider when on.
function AdvancedNum({
  label,
  value,
  def,
  range,
  onChange,
}: {
  label: string;
  value: number | null;
  def: number;
  range: NumRange;
  onChange: (v: number | null) => void;
}) {
  const overriding = value != null;
  return (
    <Box sx={{ mb: 1 }}>
      <FormControlLabel
        control={
          <Switch
            size="small"
            checked={overriding}
            onChange={(e) => onChange(e.target.checked ? def : null)}
          />
        }
        label={
          <Typography variant="body2" color="text.secondary">
            {label} {overriding ? "" : `— default (${def})`}
          </Typography>
        }
      />
      {overriding && (
        <SliderRow label={label} value={value} range={range} onChange={(v) => onChange(v as number)} />
      )}
    </Box>
  );
}

// Priority override: a select that includes an explicit "default" entry (null).
function AdvancedPriority({
  value,
  def,
  options,
  onChange,
}: {
  value: PriorityClass | null;
  def: PriorityClass;
  options: PriorityClass[];
  onChange: (v: PriorityClass | null) => void;
}) {
  return (
    <TextField
      select
      size="small"
      label="Priority"
      value={value ?? ""}
      onChange={(e) => onChange(e.target.value === "" ? null : (e.target.value as PriorityClass))}
      sx={{ mt: 1, minWidth: 200 }}
    >
      <MenuItem value="">default ({def})</MenuItem>
      {options.map((p) => (
        <MenuItem key={p} value={p}>
          {p}
        </MenuItem>
      ))}
    </TextField>
  );
}
