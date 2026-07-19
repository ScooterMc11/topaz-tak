#!/usr/bin/env python3
"""Render balanced-opening-line counts (from `topaz balancedlines`) as a single PNG.

Usage: python render_balanced_lines.py <results.json> <out.png> [layout]

`layout` (optional) is only used in combined mode (both absolute and relative bounds): "table"
(default) or "bars". Single-metric modes always use the bar+number layout.

Modes (read from the JSON `mode` field):
  * absolute  -- one count per archetype (within +-cp_bound of 0); bar+number layout.
  * relative  -- one count per archetype (within +-rel_bound of that opening's eval); bar+number.
  * combined  -- three counts per archetype: absolute, relative, and their intersection ("both").

Black Stack is the left column, 2-Komi the right. Archetypes with no lines for the shown metric are
omitted (combined mode omits archetypes with 0 absolute lines, matching the "had at least one
balanced line" rule).
"""
import json
import sys

from PIL import Image, ImageDraw, ImageFont

S = 2  # render scale (2x for crisp output)
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
CARD = "#F7F8FB"     # card fill
# Per ruleset: (mid bar, light track, dark text/intersection).
BS = {"bar": "#4F5BD5", "bg": "#E4E6FA", "txt": "#3A44B0"}
TK = {"bar": "#0E9488", "bg": "#D8F0EC", "txt": "#0A6F66"}

W = 1000
MARGIN = 44
COL_GAP = 40
HEADER_H = 150


def metric_key(mode):
    """Which per-archetype field drives a single-metric layout."""
    return {"relative": "rel", "both": "both"}.get(mode, "abs")


def rel_desc(cfg):
    """How the relative band is anchored, for the subtitle."""
    anchor = "the top move" if cfg.get("relative_to_top") else "each opening's eval"
    return f"±{cfg['rel_bound']} cp of {anchor}"


def subtitle_for(cfg, mode):
    parts = [f"depth {cfg['depth']}", f"{cfg['plies']} plies"]
    if mode == "absolute":
        parts.append(f"±{cfg['cp_bound']} cp balance bound")
    elif mode == "relative":
        parts.append(rel_desc(cfg))
    elif mode == "both":
        parts.append(f"±{cfg['cp_bound']} cp absolute AND {rel_desc(cfg)}")
    else:  # combined
        parts.append(f"±{cfg['cp_bound']} cp absolute · {rel_desc(cfg)}")
    return "   ·   ".join(parts)


def shown(rs, mode):
    arch = rs["archetypes"]
    key = metric_key(mode)
    return [a for a in arch if a[key] > 0]


def new_canvas(H):
    img = Image.new("RGB", (W * S, H * S), WHITE)
    d = ImageDraw.Draw(img)

    def T(x, y, txt, size, fill, bold=False, anchor="ls", mono=False):
        d.text((x * S, y * S), txt, font=font(size, bold, mono), fill=fill, anchor=anchor)

    def line(x1, y1, x2, y2, fill=HR, w=0.9):
        d.line([(x1 * S, y1 * S), (x2 * S, y2 * S)], fill=fill, width=round(S * w))

    def rrect(x, y, w, h, radius, fill=None, outline=None, ow=1.0):
        d.rounded_rectangle(
            [x * S, y * S, (x + w) * S, (y + h) * S],
            radius=radius * S, fill=fill, outline=outline, width=round(S * ow),
        )

    return img, d, T, line, rrect


def header(T, line, cfg, mode):
    T(MARGIN, 62, "Balanced Opening Lines", 30, INK, bold=True)
    T(MARGIN, 96, subtitle_for(cfg, mode), 16, INK2)
    T(W - MARGIN, 96, "6×6 Tak", 15, INK3, anchor="rs")
    line(MARGIN, 122, W - MARGIN, 122)


def footer(T, line, H, mode):
    fy = H - 26
    line(MARGIN, fy - 18, W - MARGIN, fy - 18)
    b = "bounds" if mode == "combined" else "bound"
    T(MARGIN, fy,
      f"Distinct end positions (rotations/reflections merged) reachable by a line whose every position stays within the {b} at the given depth.",
      11.5, INK3)


def col_geometry():
    col_w = (W - 2 * MARGIN - COL_GAP) // 2
    xs = [MARGIN, MARGIN + col_w + COL_GAP]
    return col_w, xs


# ---------------------------------------------------------------------------
# Single-metric layout (absolute or relative): bar + number per archetype.
# ---------------------------------------------------------------------------
def render_single(data, out_path):
    cfg, mode = data["config"], data["mode"]
    rulesets = data["rulesets"]
    key = metric_key(mode)
    col_w, xs = col_geometry()

    cols = []
    gmax = 1
    for (rk, pal) in [("black_stack", BS), ("two_komi", TK)]:
        rs = rulesets[rk]
        arch = shown(rs, mode)
        total = sum(a[key] for a in arch)
        cols.append((rs["label"], arch, total, pal))
        gmax = max(gmax, *[a[key] for a in arch]) if arch else gmax

    row_h = 62
    max_rows = max(len(c[1]) for c in cols) or 1
    body_top = HEADER_H + 96
    body_h = max_rows * row_h + 24
    H = body_top + body_h + 56

    img, d, T, line, rrect = new_canvas(H)
    header(T, line, cfg, mode)

    for (cx, (label, arch, total, pal)) in zip(xs, cols):
        rrect(cx, HEADER_H - 8, col_w, body_top + body_h - HEADER_H + 8, 14, fill=CARD, outline=HR)
        pad = 22
        ix, iw = cx + pad, col_w - 2 * pad
        T(ix, HEADER_H + 46, label, 21, pal["txt"], bold=True)
        T(cx + col_w - pad, HEADER_H + 40, f"{total:,}", 30, INK, bold=True, anchor="rs")
        T(cx + col_w - pad, HEADER_H + 62, "distinct end positions", 12, INK3, anchor="rs")
        line(ix, HEADER_H + 78, cx + col_w - pad, HEADER_H + 78)

        ry = body_top + 18
        if not arch:
            T(ix, ry + 20, "No balanced lines at this bound.", 15, INK3)
            continue
        for a in arch:
            cnt = a[key]
            T(ix, ry, a["name"], 15, INK)
            T(cx + col_w - pad, ry, f"{cnt:,}", 15, pal["txt"], bold=True, anchor="rs")
            by = ry + 12
            rrect(ix, by, iw, 13, 6, fill=pal["bg"])
            rrect(ix, by, max(6, iw * (cnt / gmax)), 13, 6, fill=pal["bar"])
            ry += row_h

    footer(T, line, H, mode)
    img.resize((W, H), Image.LANCZOS).save(out_path)
    print(f"wrote {out_path}")


# ---------------------------------------------------------------------------
# Combined layout A: compact numeric table (Absolute / Relative / Both columns).
# ---------------------------------------------------------------------------
def render_combined_table(data, out_path):
    cfg, mode = data["config"], data["mode"]
    rulesets = data["rulesets"]
    col_w, xs = col_geometry()

    cols = []
    for (rk, pal) in [("black_stack", BS), ("two_komi", TK)]:
        rs = rulesets[rk]
        arch = shown(rs, mode)
        tot = {m: sum(a[m] for a in arch) for m in ("abs", "rel", "both")}
        cols.append((rs["label"], arch, tot, pal))

    row_h = 34
    max_rows = max(len(c[1]) for c in cols) or 1
    body_top = HEADER_H + 92
    body_h = 26 + max_rows * row_h + 12 + 34  # colhead + rows + rule + totals
    H = body_top + body_h + 56

    img, d, T, line, rrect = new_canvas(H)
    header(T, line, cfg, mode)

    # three numeric sub-columns (right edges) within each card
    def subcols(cx, pad):
        right = cx + col_w - pad
        step = 88
        return [right - 2 * step, right - step, right]  # abs, rel, both right-edges

    for (cx, (label, arch, tot, pal)) in zip(xs, cols):
        rrect(cx, HEADER_H - 8, col_w, body_top + body_h - HEADER_H + 8, 14, fill=CARD, outline=HR)
        pad = 22
        ix = cx + pad
        T(ix, HEADER_H + 44, label, 21, pal["txt"], bold=True)

        cxs = subcols(cx, pad)
        chy = body_top - 6
        for lab, rx in zip(["Absolute", "Relative", "Both"], cxs):
            T(rx, chy, lab, 11.5, INK3, anchor="rs")
        line(ix, chy + 8, cx + col_w - pad, chy + 8)

        ry = body_top + 30
        if not arch:
            T(ix, ry, "No balanced lines at this bound.", 15, INK3)
        for a in arch:
            T(ix, ry, a["name"], 14.5, INK)
            for m, rx in zip(("abs", "rel", "both"), cxs):
                col = pal["txt"] if m == "both" else INK
                bold = (m == "both")
                T(rx, ry, f"{a[m]:,}", 14.5, col, bold=bold, anchor="rs")
            ry += row_h

        # totals row
        ry += 2
        line(ix, ry - 20, cx + col_w - pad, ry - 20)
        T(ix, ry, "Total", 14.5, INK, bold=True)
        for m, rx in zip(("abs", "rel", "both"), cxs):
            col = pal["txt"] if m == "both" else INK
            T(rx, ry, f"{tot[m]:,}", 14.5, col, bold=True, anchor="rs")

    footer(T, line, H, mode)
    img.resize((W, H), Image.LANCZOS).save(out_path)
    print(f"wrote {out_path}")


# ---------------------------------------------------------------------------
# Combined layout B: grouped bars. Two bars per archetype (absolute, relative), each with its
# intersection ("both") prefix drawn in the darker shade so the shared subset lines up.
# ---------------------------------------------------------------------------
def render_combined_bars(data, out_path):
    cfg, mode = data["config"], data["mode"]
    rulesets = data["rulesets"]
    col_w, xs = col_geometry()

    cols = []
    gmax = 1
    for (rk, pal) in [("black_stack", BS), ("two_komi", TK)]:
        rs = rulesets[rk]
        arch = shown(rs, mode)
        tot = {m: sum(a[m] for a in arch) for m in ("abs", "rel", "both")}
        cols.append((rs["label"], arch, tot, pal))
        for a in arch:
            gmax = max(gmax, a["abs"], a["rel"])

    row_h = 82
    max_rows = max(len(c[1]) for c in cols) or 1
    body_top = HEADER_H + 96
    body_h = max_rows * row_h + 20
    H = body_top + body_h + 72

    img, d, T, line, rrect = new_canvas(H)
    header(T, line, cfg, mode)

    # legend under header
    lx = MARGIN
    T(lx, 140, "bars: absolute (top) & relative (bottom); darker = intersection", 11.5, INK3)

    def bar(ix, y, iw, val, both, pal):
        rrect(ix, y, iw, 12, 6, fill=pal["bg"])
        w_val = max(2, iw * (val / gmax)) if gmax else 2
        rrect(ix, y, w_val, 12, 6, fill=pal["bar"])
        if both > 0:
            w_both = max(2, iw * (both / gmax))
            rrect(ix, y, min(w_both, w_val), 12, 6, fill=pal["txt"])

    for (cx, (label, arch, tot, pal)) in zip(xs, cols):
        rrect(cx, HEADER_H - 8, col_w, body_top + body_h - HEADER_H + 8, 14, fill=CARD, outline=HR)
        pad = 22
        ix, iw = cx + pad, col_w - 2 * pad
        T(ix, HEADER_H + 46, label, 21, pal["txt"], bold=True)
        T(cx + col_w - pad, HEADER_H + 40, f"{tot['abs']:,} / {tot['rel']:,}", 22, INK, bold=True, anchor="rs")
        T(cx + col_w - pad, HEADER_H + 60, "abs / rel totals", 11.5, INK3, anchor="rs")
        line(ix, HEADER_H + 78, cx + col_w - pad, HEADER_H + 78)

        ry = body_top + 16
        if not arch:
            T(ix, ry + 16, "No balanced lines at this bound.", 15, INK3)
        for a in arch:
            T(ix, ry, a["name"], 14.5, INK)
            T(cx + col_w - pad, ry,
              f"A {a['abs']:,}   R {a['rel']:,}   B {a['both']:,}", 12.5, INK2, anchor="rs")
            bar(ix, ry + 10, iw, a["abs"], a["both"], pal)
            bar(ix, ry + 28, iw, a["rel"], a["both"], pal)
            ry += row_h

    footer(T, line, H, mode)
    img.resize((W, H), Image.LANCZOS).save(out_path)
    print(f"wrote {out_path}")


def render(data, out_path, layout):
    mode = data["mode"]
    if mode in ("absolute", "relative", "both"):
        render_single(data, out_path)
    elif mode == "combined":
        if layout == "bars":
            render_combined_bars(data, out_path)
        else:
            render_combined_table(data, out_path)
    else:
        raise SystemExit(f"unknown mode: {mode}")


def main():
    if len(sys.argv) not in (3, 4):
        print("Usage: python render_balanced_lines.py <results.json> <out.png> [table|bars]", file=sys.stderr)
        sys.exit(2)
    layout = sys.argv[3] if len(sys.argv) == 4 else "table"
    with open(sys.argv[1], "r", encoding="utf-8") as f:
        data = json.load(f)
    render(data, sys.argv[2], layout)


if __name__ == "__main__":
    main()
