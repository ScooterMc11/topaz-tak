// Two horizontal box plots on a shared ELO axis: the opponent ratings this bot has WON against,
// and the ones it has LOST against. Both come from games matching the bot's *current* configuration,
// so changing a tunable starts the sample over.
//
// Drawn with positioned divs rather than SVG so it scales to any container width without stretching
// stroke widths — a box plot is only rects and lines, and percentage offsets do that natively.
import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import Tooltip from "@mui/material/Tooltip";
import { useTheme } from "@mui/material/styles";

export interface BoxStats {
  n: number;
  min: number;
  q1: number;
  median: number;
  q3: number;
  max: number;
  whiskerLo: number;
  whiskerHi: number;
  outliers: number[];
}

// Linear-interpolation quantile (the "type 7" definition used by R, NumPy and Excel).
function quantile(sorted: number[], p: number): number {
  if (sorted.length === 1) return sorted[0];
  const idx = (sorted.length - 1) * p;
  const lo = Math.floor(idx);
  const hi = Math.ceil(idx);
  return sorted[lo] + (sorted[hi] - sorted[lo]) * (idx - lo);
}

// Tukey box: the whiskers stop at the most extreme value still within 1.5 IQR of the box, and
// anything beyond is drawn as an outlier dot.
export function boxStats(values: number[]): BoxStats | null {
  if (values.length === 0) return null;
  const s = [...values].sort((a, b) => a - b);
  const q1 = quantile(s, 0.25);
  const median = quantile(s, 0.5);
  const q3 = quantile(s, 0.75);
  const iqr = q3 - q1;
  const loFence = q1 - 1.5 * iqr;
  const hiFence = q3 + 1.5 * iqr;
  const inner = s.filter((v) => v >= loFence && v <= hiFence);
  return {
    n: s.length,
    min: s[0],
    q1,
    median,
    q3,
    max: s[s.length - 1],
    whiskerLo: inner.length ? inner[0] : s[0],
    whiskerHi: inner.length ? inner[inner.length - 1] : s[s.length - 1],
    outliers: s.filter((v) => v < loFence || v > hiFence),
  };
}

const ROW_H = 26;
const BOX_H = 14;

interface PlotRowProps {
  label: string;
  color: string;
  stats: BoxStats | null;
  domain: [number, number];
}

function PlotRow({ label, color, stats, domain }: PlotRowProps) {
  const [lo, hi] = domain;
  const span = hi - lo || 1;
  const pct = (v: number) => ((v - lo) / span) * 100;

  const summary = stats
    ? `${label}: n=${stats.n} · min ${Math.round(stats.min)} · Q1 ${Math.round(stats.q1)} · ` +
      `median ${Math.round(stats.median)} · Q3 ${Math.round(stats.q3)} · max ${Math.round(stats.max)}`
    : `${label}: no rated opponents yet on this configuration`;

  return (
    <Tooltip title={summary} placement="top">
      <Box sx={{ display: "flex", alignItems: "center", gap: 1, minWidth: 0 }}>
        <Typography
          variant="caption"
          sx={{ width: 40, flexShrink: 0, color, fontWeight: 700, textAlign: "right" }}
        >
          {label}
        </Typography>
        <Box sx={{ position: "relative", flexGrow: 1, height: ROW_H, minWidth: 0 }}>
          {/* Baseline so an empty distribution still reads as a track rather than a gap. */}
          <Box
            sx={{
              position: "absolute",
              top: ROW_H / 2,
              left: 0,
              right: 0,
              borderTop: 1,
              borderColor: "divider",
            }}
          />
          {stats && (
            <>
              {/* Whisker spine + caps */}
              <Box
                sx={{
                  position: "absolute",
                  top: ROW_H / 2,
                  height: 0,
                  left: `${pct(stats.whiskerLo)}%`,
                  width: `${pct(stats.whiskerHi) - pct(stats.whiskerLo)}%`,
                  borderTop: "1px solid",
                  borderColor: color,
                }}
              />
              {[stats.whiskerLo, stats.whiskerHi].map((v, i) => (
                <Box
                  key={i}
                  sx={{
                    position: "absolute",
                    top: (ROW_H - BOX_H) / 2 + 3,
                    height: BOX_H - 6,
                    left: `${pct(v)}%`,
                    width: 0,
                    borderLeft: "1px solid",
                    borderColor: color,
                  }}
                />
              ))}
              {/* Interquartile box */}
              <Box
                sx={{
                  position: "absolute",
                  top: (ROW_H - BOX_H) / 2,
                  height: BOX_H,
                  left: `${pct(stats.q1)}%`,
                  width: `${Math.max(pct(stats.q3) - pct(stats.q1), 0.4)}%`,
                  bgcolor: color,
                  opacity: 0.28,
                  border: "1px solid",
                  borderColor: color,
                  borderRadius: 0.5,
                  boxSizing: "border-box",
                }}
              />
              {/* Median */}
              <Box
                sx={{
                  position: "absolute",
                  top: (ROW_H - BOX_H) / 2,
                  height: BOX_H,
                  left: `${pct(stats.median)}%`,
                  width: 0,
                  borderLeft: "2px solid",
                  borderColor: color,
                }}
              />
              {stats.outliers.map((v, i) => (
                <Box
                  key={`o${i}`}
                  sx={{
                    position: "absolute",
                    top: ROW_H / 2 - 2.5,
                    left: `calc(${pct(v)}% - 2.5px)`,
                    width: 5,
                    height: 5,
                    borderRadius: "50%",
                    bgcolor: color,
                    opacity: 0.7,
                  }}
                />
              ))}
            </>
          )}
        </Box>
        <Typography
          variant="caption"
          color="text.secondary"
          sx={{ width: 34, flexShrink: 0, textAlign: "right", fontVariantNumeric: "tabular-nums" }}
        >
          {stats ? `n=${stats.n}` : "n=0"}
        </Typography>
      </Box>
    </Tooltip>
  );
}

interface EloBoxPlotsProps {
  won: number[];
  lost: number[];
  matchedGames: number;
  pendingRatings: number;
}

export default function EloBoxPlots({
  won,
  lost,
  matchedGames,
  pendingRatings,
}: EloBoxPlotsProps) {
  const theme = useTheme();
  const wonStats = boxStats(won);
  const lostStats = boxStats(lost);

  // One axis for both rows — the whole point is comparing where the two distributions sit.
  const all = [...won, ...lost];
  const rawLo = all.length ? Math.min(...all) : 1000;
  const rawHi = all.length ? Math.max(...all) : 2000;
  const pad = Math.max((rawHi - rawLo) * 0.08, 25);
  const domain: [number, number] = [Math.floor(rawLo - pad), Math.ceil(rawHi + pad)];
  const mid = Math.round((domain[0] + domain[1]) / 2);

  if (all.length === 0) {
    return (
      <Typography variant="caption" color="text.secondary">
        {matchedGames === 0
          ? "No games on this configuration yet — the opponent-ELO plots start over whenever you change a bot's settings."
          : pendingRatings > 0
            ? `${matchedGames} game${matchedGames > 1 ? "s" : ""} on this configuration; still looking up opponent ratings.`
            : `${matchedGames} game${matchedGames > 1 ? "s" : ""} on this configuration, none with a rated opponent.`}
      </Typography>
    );
  }

  return (
    <Box sx={{ minWidth: 0 }}>
      <PlotRow label="Won" color={theme.palette.success.main} stats={wonStats} domain={domain} />
      <PlotRow label="Lost" color={theme.palette.error.main} stats={lostStats} domain={domain} />
      {/* Shared axis */}
      <Box sx={{ display: "flex", alignItems: "center", gap: 1, mt: 0.25 }}>
        <Box sx={{ width: 40, flexShrink: 0 }} />
        <Box
          sx={{
            flexGrow: 1,
            minWidth: 0,
            display: "flex",
            justifyContent: "space-between",
            color: "text.secondary",
          }}
        >
          {[domain[0], mid, domain[1]].map((v) => (
            <Typography key={v} variant="caption" sx={{ fontSize: 10 }}>
              {v}
            </Typography>
          ))}
        </Box>
        <Box sx={{ width: 34, flexShrink: 0 }} />
      </Box>
      {pendingRatings > 0 && (
        <Typography variant="caption" color="text.secondary" sx={{ fontSize: 10 }}>
          {pendingRatings} game{pendingRatings > 1 ? "s" : ""} still awaiting an opponent rating.
        </Typography>
      )}
    </Box>
  );
}
