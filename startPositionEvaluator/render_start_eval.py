#!/usr/bin/env python3
"""Render start-position eval-vs-depth curves (from `topaz starteval`) as a single PNG.

Usage: python render_start_eval.py <results.json> <out.png>

Two panels side by side: Black Stack (left), 2-Komi (right). Each panel has
  1. a line chart -- x = search depth, y = eval (human cp), one colored line per opening archetype;
  2. a table below it -- one row per archetype, one column per depth, cell = the cp value,
     with the table's depth columns aligned exactly under the chart's x-axis.
Each archetype keeps the same color in the chart, in the table row label, and across both rulesets.
Both panels share one cp (y) scale so the rulesets are directly comparable. cp is already
human-facing (engine internal / 4), White-positive; depth 0 is the static eval.
"""
import json
import sys

from PIL import Image, ImageDraw, ImageFont

S = 2  # render scale (2x, downsampled at save for crisp text)
FONTDIR = "C:/Windows/Fonts/"
_fc = {}


def font(size, bold=False, mono=False):
    k = (size, bold, mono)
    if k not in _fc:
        name = "consola.ttf" if mono else ("arialbd.ttf" if bold else "arial.ttf")
        try:
            _fc[k] = ImageFont.truetype(FONTDIR + name, round(size * S))
        except OSError:
            try:
                _fc[k] = ImageFont.truetype("DejaVuSans.ttf", round(size * S))
            except OSError:
                _fc[k] = ImageFont.load_default()
    return _fc[k]


# Palette (light-background PNG).
WHITE = "#FFFFFF"
INK = "#1F2430"      # primary text
INK2 = "#5A6072"     # secondary text
INK3 = "#8A90A0"     # tertiary / small print
HR = "#E6E8EF"       # hairlines
GRID = "#EEF0F6"     # chart gridlines
CARD = "#F7F8FB"     # card fill
ZERO = "#B6BCCC"     # y = 0 reference line

# Per-archetype color, CONSISTENT across both rulesets. Keyed by archetype name.
ARCH_COLORS = {
    "diagonal corners": "#2563EB",  # blue
    "adjacent corners": "#EA580C",  # orange
    "hug": "#16A34A",               # green
    "gap hug": "#9333EA",           # purple
}
FALLBACK = "#64748B"

# Ruleset accent (panel titles).
RS_ACCENT = {"black_stack": "#3A44B0", "two_komi": "#0A6F66"}

MATE = 1000.0        # |cp| >= this is treated as a solved win/loss for display + clamped in the chart
YCLAMP = 200.0       # clamp cp to +-this when computing the shared y-range (keeps a mate off-scale)

MARGIN = 40
PANEL_GAP = 46
GUTTER = 150         # left gutter per panel: y-axis labels + table row names
RIGHT_PAD = 24
COL_STEP = 34        # logical px between adjacent depth columns (== chart x spacing)
CHART_H = 300
ROW_H = 26
HEADER_H = 96


def color_for(name):
    return ARCH_COLORS.get(name, FALLBACK)


def fmt_cell(cp):
    """Table cell: whole-number cp (rounded), with a clean '0' and no '-0'."""
    if cp >= MATE:
        return "+W"
    if cp <= -MATE:
        return "-L"
    v = int(round(cp))
    if v == 0:
        return "0"
    return f"{v:+d}"


def load_panels(data):
    """Return [(rs_key, label, komi, [archetype...]), ...] and the overall depth + cp ranges."""
    panels = []
    all_depths = set()
    cps = []
    for rk in ("black_stack", "two_komi"):
        rs = data["rulesets"][rk]
        arch = []
        for a in rs["archetypes"]:
            pts = {int(p["depth"]): float(p["cp"]) for p in a["points"]}
            arch.append({"name": a["name"], "opening": a.get("opening", ""), "pts": pts})
            all_depths.update(pts.keys())
            cps.extend(pts.values())
        panels.append((rk, rs["label"], rs.get("komi", 0), arch))
    dmin, dmax = (min(all_depths), max(all_depths)) if all_depths else (0, 0)
    clamped = [max(-YCLAMP, min(YCLAMP, c)) for c in cps] or [0.0]
    lo, hi = min(clamped), max(clamped)
    if hi - lo < 8:  # give a thin range some breathing room
        mid = (hi + lo) / 2
        lo, hi = mid - 6, mid + 6
    pad = (hi - lo) * 0.12
    return panels, dmin, dmax, lo - pad, hi + pad


def main():
    if len(sys.argv) != 3:
        print("Usage: python render_start_eval.py <results.json> <out.png>", file=sys.stderr)
        sys.exit(2)
    with open(sys.argv[1], "r", encoding="utf-8") as f:
        data = json.load(f)
    out_path = sys.argv[2]

    cfg = data["config"]
    panels, dmin, dmax, ylo, yhi = load_panels(data)
    depths = list(range(dmin, dmax + 1))
    n = len(depths)
    max_rows = max((len(p[3]) for p in panels), default=1)

    # ---- geometry ----
    plot_w = max(COL_STEP, (n - 1) * COL_STEP)
    panel_w = GUTTER + plot_w + RIGHT_PAD
    W = MARGIN * 2 + panel_w * 2 + PANEL_GAP

    chart_top = HEADER_H + 34
    chart_bottom = chart_top + CHART_H
    table_top = chart_bottom + 62          # room for x-axis label
    table_header_y = table_top
    table_rows_top = table_top + 24
    table_bottom = table_rows_top + max_rows * ROW_H
    H = int(table_bottom + 54)

    img = Image.new("RGB", (W * S, H * S), WHITE)
    d = ImageDraw.Draw(img)

    def T(x, y, txt, size, fill, bold=False, anchor="ls", mono=False):
        d.text((x * S, y * S), txt, font=font(size, bold, mono), fill=fill, anchor=anchor)

    def line(x1, y1, x2, y2, fill=HR, w=1.0):
        d.line([(x1 * S, y1 * S), (x2 * S, y2 * S)], fill=fill, width=max(1, round(S * w)))

    def rrect(x, y, w, h, radius, fill=None, outline=None, ow=1.0):
        d.rounded_rectangle(
            [x * S, y * S, (x + w) * S, (y + h) * S],
            radius=radius * S, fill=fill, outline=outline, width=round(S * ow),
        )

    def dot(x, y, r, fill):
        d.ellipse([(x - r) * S, (y - r) * S, (x + r) * S, (y + r) * S], fill=fill)

    # ---- header ----
    T(MARGIN, 56, "Opening Archetype Evaluation vs. Search Depth", 27, INK, bold=True)
    T(W - MARGIN, 58, "6x6", 15, INK3, anchor="rs")
    line(MARGIN, 80, W - MARGIN, 80)

    panel_x0 = [MARGIN, MARGIN + panel_w + PANEL_GAP]

    def x_of(depth):
        if n <= 1:
            return 0.0
        return (depth - dmin) / (dmax - dmin) * plot_w

    def y_of(cp):
        c = max(ylo, min(yhi, cp))
        return chart_bottom - (c - ylo) / (yhi - ylo) * CHART_H

    # nice y gridline steps
    def y_ticks():
        span = yhi - ylo
        step = 5
        for s in (5, 10, 20, 25, 50, 100):
            if span / s <= 8:
                step = s
                break
        t = (int(ylo // step)) * step
        ticks = []
        while t <= yhi:
            if t >= ylo:
                ticks.append(t)
            t += step
        return ticks

    # x label stride so chart-axis numbers don't crowd (table header shows every depth anyway)
    stride = 1
    while n / stride > 26:
        stride += 1

    for (px, (rk, label, komi, arch)) in zip(panel_x0, panels):
        plot_left = px + GUTTER
        accent = RS_ACCENT.get(rk, INK)

        # panel title
        T(px, chart_top - 14, label, 20, accent, bold=True)

        # chart card
        rrect(plot_left - 10, chart_top - 6, plot_w + 20, CHART_H + 12, 10, fill=CARD, outline=HR)

        # y gridlines + labels
        for tv in y_ticks():
            yy = y_of(tv)
            is_zero = (tv == 0)
            line(plot_left, yy, plot_left + plot_w, yy, fill=(ZERO if is_zero else GRID),
                 w=1.4 if is_zero else 1.0)
            T(plot_left - 8, yy + 4, f"{tv:g}", 11.5, (INK2 if is_zero else INK3), anchor="rs")
        T(plot_left - 8, chart_top - 12, "cp", 11.5, INK3, anchor="rs")

        # x gridlines + (strided) labels
        for i, dep in enumerate(depths):
            xx = plot_left + x_of(dep)
            line(xx, chart_top, xx, chart_bottom, fill=GRID, w=1.0)
            if i % stride == 0:
                T(xx, chart_bottom + 16, str(dep), 11, INK3, anchor="ms")
        T(plot_left + plot_w / 2, chart_bottom + 40, "search depth", 12.5, INK2, anchor="ms")

        # lines + markers per archetype
        for a in arch:
            col = color_for(a["name"])
            seq = [(dep, a["pts"][dep]) for dep in depths if dep in a["pts"]]
            if not seq:
                continue
            poly = [((plot_left + x_of(dep)) * S, y_of(cp) * S) for dep, cp in seq]
            if len(poly) >= 2:
                d.line(poly, fill=col, width=max(1, round(S * 1.8)), joint="curve")
            for dep, cp in seq:
                dot(plot_left + x_of(dep), y_of(cp), 2.4, col)

        # legend (top-left inside chart)
        ly = chart_top + 12
        for a in arch:
            col = color_for(a["name"])
            line(plot_left + 6, ly, plot_left + 24, ly, fill=col, w=2.2)
            dot(plot_left + 15, ly, 2.2, col)
            T(plot_left + 30, ly + 4, a["name"], 11.5, INK, anchor="ls")
            ly += 17

        # ---- table ----
        # header row: depth numbers, aligned to chart x
        T(px, table_header_y + 4, "depth", 11.5, INK2, bold=True, anchor="ls")
        for dep in depths:
            xx = plot_left + x_of(dep)
            T(xx, table_header_y + 4, str(dep), 10.5, INK2, bold=True, anchor="ms")
        line(px, table_rows_top - 8, px + panel_w - RIGHT_PAD, table_rows_top - 8, fill=HR)

        # one row per archetype
        for r, a in enumerate(arch):
            ry = table_rows_top + r * ROW_H + ROW_H / 2
            col = color_for(a["name"])
            if r % 2 == 1:
                rrect(px, table_rows_top + r * ROW_H, panel_w - RIGHT_PAD, ROW_H, 0, fill=CARD)
            T(px, ry + 4, a["name"], 11, col, bold=True, anchor="ls")
            for dep in depths:
                if dep in a["pts"]:
                    xx = plot_left + x_of(dep)
                    T(xx, ry + 4, fmt_cell(a["pts"][dep]), 10, INK, anchor="ms")

    T(MARGIN, H - 22,
      "Positive cp favors White. Each curve reads one iterative-deepening search per archetype; "
      "table columns align with the chart's depth axis above.",
      11, INK3)

    img.resize((W, H), Image.LANCZOS).save(out_path)
    print(f"wrote {out_path}")


if __name__ == "__main__":
    main()
