# Renders the balance info sheets as PNGs using Pillow.
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
            _fc[k] = ImageFont.truetype(FONTDIR + "arial.ttf", round(size * S))
    return _fc[k]

BLUE_F, BLUE_T = "#3B82C4", "#185FA5"
RED_F,  RED_T  = "#D14343", "#A32D2D"
BLUE_L,  RED_L  = "#BBD7F5", "#F4BCBC"   # flat region (lightest)
BLUE_RD, RED_RD = "#85B7EB", "#F09595"   # road region (lighter than the result bar, darker than flat)
C_D, D_T = "#CFCDC4", "#4A4944"
T1, T2, T3 = "#2C2C2A", "#5F5E5A", "#8A897F"
HR, WHITE = "#E7E5DD", "#FFFFFF"
def gc(g): return BLUE_T if g >= 0 else RED_T

def make_sheet(cfg, out):
    rows = cfg["rows"]
    nrow = len(rows)
    y0, rh, Wl = 360, 144, 800
    Hl = y0 + nrow * rh + 16
    img = Image.new("RGB", (Wl * S, Hl * S), WHITE)
    d = ImageDraw.Draw(img)
    def tw(txt, size, bold=False, mono=False):
        return d.textlength(txt, font=font(size, bold, mono)) / S
    def T(x, y, txt, size, fill, bold=False, anchor="ls", mono=False):
        d.text((x * S, y * S), txt, font=font(size, bold, mono), fill=fill, anchor=anchor)
    def line(x1, y1, x2, y2):
        d.line([(x1 * S, y1 * S), (x2 * S, y2 * S)], fill=HR, width=round(S * 0.8))
    def rect(x, y, w, h, fill):
        d.rectangle([x * S, y * S, (x + w) * S, (y + h) * S], fill=fill)
    def comp_bar(x, y, w, h, wp, dp, bp, fs):
        cx = x
        for pct, col, tc in [(wp, BLUE_F, WHITE), (dp, C_D, D_T), (bp, RED_F, WHITE)]:
            sw = w * pct / 100.0
            rect(cx, y, sw, h, col)
            if sw > 30:
                d.text(((cx + sw / 2) * S, (y + h / 2) * S), f"{pct:.1f}%", font=font(fs), fill=tc, anchor="mm")
            cx += sw
    def rf_bar(x, y, w, h, road, solid, light, stxt, ltxt):
        rw = w * road / 100.0
        rect(x, y, rw, h, solid)
        rect(x + rw, y, w - rw, h, light)
        d.text(((x + rw / 2) * S, (y + h / 2) * S), f"{road}%", font=font(11.5), fill=stxt, anchor="mm")
        d.text(((x + rw + (w - rw) / 2) * S, (y + h / 2) * S), f"{100-road}%", font=font(11.5), fill=ltxt, anchor="mm")
    def rf_section(x, y, w, wr, br):
        T(x, y, "Road", 10.5, T3)
        T(x + w, y, "Flat", 10.5, T3, anchor="rs")
        rf_bar(x, y + 7, w, 17, wr, BLUE_RD, BLUE_L, BLUE_T, BLUE_T)
        rf_bar(x, y + 28, w, 17, br, RED_RD, RED_L, RED_T, RED_T)

    d.rounded_rectangle([S, S, (Wl - 1) * S, (Hl - 1) * S], radius=16 * S, outline=HR, width=round(S))
    T(40, 56, cfg["title"], 25, T1, bold=True)
    if cfg.get("engine"):  # optional engine label, top-right
        T(760, 56, cfg["engine"], 14, T2, anchor="rs")
    T(40, 83, cfg["subtitle"], 15, T2)
    line(40, 104, 760, 104)
    lx = 40
    for lab, col in [("White wins", BLUE_F), ("Draws", C_D), ("Black wins", RED_F)]:
        rect(lx, 124, 11, 11, col)
        T(lx + 17, 133, lab, 12.5, T2)
        lx += 17 + tw(lab, 12.5) + 22

    T(40, 164, "Overall (weighted)", 13, T2)
    o = cfg["overall"]
    comp_bar(40, 176, 720, 46, o["w"], o["d"], o["b"], 16)
    T(40, 252, "White−Black ", 14, T1)
    T(40 + tw("White−Black ", 14), 252, f"{o['gap']:+.1f}%", 16, gc(o["gap"]), bold=True)
    rf_section(40, 274, 720, o["wr"], o["br"])
    line(40, 326, 760, 326)

    T(40, 354, "By opening archetype", 13, T2)
    for i, r in enumerate(rows):
        name, cnt, share, wp, dp, bp, gap, ar, wr, br, ptn = r
        yr = y0 + i * rh
        T(40, yr + 24, name, 15, T1)
        T(40, yr + 43, f"{cnt:,} games · {share:g}%", 12, T3)
        T(40, yr + 61, "e.g. ", 11.5, T3)
        T(40 + tw("e.g. ", 11.5), yr + 61, ptn, 11.5, T2, mono=True)
        comp_bar(210, yr + 8, 420, 38, wp, dp, bp, 14)
        T(760, yr + 26, "White−Black", 11, T3, anchor="rs")
        T(760, yr + 48, f"{gap:+.1f}%", 19, gc(gap), bold=True, anchor="rs")
        rf_section(210, yr + 64, 420, wr, br)
        if i < nrow - 1:
            dy = yr + (rh + 117) // 2  # centered in the gap below this chunk's content
            line(40, dy, 760, dy)
    img.save(out)
    print("saved", out, img.size)

SUB = "6×6 board  ·  50,000 self-play games  ·  60,000 nodes per move"
black_stack = {
    "title": "Double Black Stack Self-Play Balance", "subtitle": SUB,
    "overall": {"w":35.7,"d":29.9,"b":34.4,"gap":1.3,"ar":32,"wr":31,"br":33},
    "rows": [
        ("Diagonal Corners",10000,20,41.1,30.6,28.3,12.8,31,31,33,"2a1 f6"),
        ("Adjacent Corners",10000,20,38.2,29.7,32.1,6.1,32,30,35,"2a1 a6"),
        ("Hug",15000,30,33.7,30.1,36.2,-2.6,33,31,34,"2a1 b1"),
        ("Gap Hug",10000,20,39.8,30.5,29.7,10.0,31,30,32,"2a1 c1"),
        ("Center Stack",5000,10,17.6,27.3,55.1,-37.5,31,36,30,"2c3 a1"),
    ],
}
two_komi = {
    "title": "2 Komi Self-Play Balance", "subtitle": SUB,
    "overall": {"w":40.3,"d":34.4,"b":25.3,"gap":15.0,"ar":33,"wr":39,"br":24},
    "rows": [
        ("Diagonal Corners",20000,40,39.0,34.4,26.6,12.4,34,40,24,"a1 f6"),
        ("Adjacent Corners",20000,40,37.8,35.5,26.7,11.1,33,39,23,"a1 a6"),
        ("Hug",10000,20,47.8,32.3,19.9,27.8,34,38,24,"a1 b1"),
    ],
}
black_stack_sans = {
    "title": "Double Black Stack Self-Play Balance",
    "subtitle": "6×6 board  ·  45,000 self-play games  ·  60,000 nodes per move  ·  excludes center-stack opening",
    "overall": {"w":37.7,"d":30.2,"b":32.1,"gap":5.6,"ar":32,"wr":30,"br":34},
    "rows": [
        ("Diagonal Corners",10000,22.2,41.1,30.6,28.3,12.8,31,31,33,"2a1 f6"),
        ("Adjacent Corners",10000,22.2,38.2,29.7,32.1,6.1,32,30,35,"2a1 a6"),
        ("Hug",15000,33.3,33.7,30.1,36.2,-2.6,33,31,34,"2a1 b1"),
        ("Gap Hug",10000,22.2,39.8,30.5,29.7,10.0,31,30,32,"2a1 c1"),
    ],
}
two_komi_sans = {
    "title": "2 Komi Self-Play Balance",
    "subtitle": "6×6 board  ·  40,000 self-play games  ·  60,000 nodes per move  ·  excludes hug opening",
    "overall": {"w":38.4,"d":34.9,"b":26.7,"gap":11.7,"ar":33,"wr":40,"br":23},
    "rows": [
        ("Diagonal Corners",20000,50,39.0,34.4,26.6,12.4,34,40,24,"a1 f6"),
        ("Adjacent Corners",20000,50,37.8,35.5,26.7,11.1,33,39,23,"a1 a6"),
    ],
}
# ── 5×5 ────────────────────────────────────────────────────────────────────
# Black-stack (double-black-stack opening, komi 0), bs-v3 net, both colours per opening.
black_stack_5s = {
    "title": "Double Black Stack Self-Play Balance",
    "subtitle": "5×5 board  ·  20,000 self-play games  ·  15,000 nodes per move",
    "overall": {"w":38.4,"d":23.6,"b":38.0,"gap":0.4,"ar":20,"wr":43,"br":43},
    "rows": [
        ("Diagonal Corners",4000,20,44.5,23.4,32.0,12.5,20,41,42,"2a1 e5"),
        ("Adjacent Corners",4000,20,42.4,23.1,34.5,7.9,20,45,44,"2a1 a5"),
        ("Hug",4000,20,37.0,23.9,39.1,-2.1,20,41,42,"2a1 b1"),
        ("Gap Hug",4000,20,40.7,23.7,35.6,5.1,20,43,46,"2a1 c1"),
        ("Center Stack",4000,20,27.2,24.2,48.6,-21.4,20,45,42,"2c3 a1"),
    ],
}

# Same data, excluding the center-stack opening (the −21.4% outlier); overall recomputed over the
# remaining 4 archetypes (16,000 games): W 6587 / D 3760 / B 5653.
black_stack_5s_sans = {
    "title": "Double Black Stack Self-Play Balance",
    "subtitle": "5×5 board  ·  16,000 self-play games  ·  15,000 nodes per move  ·  excludes center-stack opening",
    "overall": {"w":41.2,"d":23.5,"b":35.3,"gap":5.8,"ar":25,"wr":43,"br":43},
    "rows": [
        ("Diagonal Corners",4000,25,44.5,23.4,32.0,12.5,25,41,42,"2a1 e5"),
        ("Adjacent Corners",4000,25,42.4,23.1,34.5,7.9,25,45,44,"2a1 a5"),
        ("Hug",4000,25,37.0,23.9,39.1,-2.1,25,41,42,"2a1 b1"),
        ("Gap Hug",4000,25,40.7,23.7,35.6,5.1,25,43,46,"2a1 c1"),
    ],
}

# Standard rules (single-flat opening), no komi, same bs-v3 net — the known-imbalanced baseline.
standard_5s = {
    "title": "Standard Self-Play Balance",
    "subtitle": "5×5 board  ·  20,000 self-play games  ·  15,000 nodes per move  ·  no komi",
    "overall": {"w":88.2,"d":7.2,"b":4.6,"gap":83.6,"ar":33,"wr":41,"br":60},
    "rows": [
        ("Diagonal Corners",6666,33.3,86.2,8.2,5.6,80.6,33,42,63,"a1 e5"),
        ("Adjacent Corners",6667,33.3,86.7,8.1,5.2,81.5,33,42,58,"a1 a5"),
        ("Hug",6667,33.3,91.6,5.4,2.9,88.7,33,40,60,"a1 b1"),
    ],
}
# Standard, excluding hug (the +88.7% outlier); overall recomputed over diagonal + adjacent
# (13,333 games): W 11527 / D 1082 / B 724.
standard_5s_sans = {
    "title": "Standard Self-Play Balance",
    "subtitle": "5×5 board  ·  13,333 self-play games  ·  15,000 nodes per move  ·  no komi  ·  excludes hug opening",
    "overall": {"w":86.5,"d":8.1,"b":5.4,"gap":81.0,"ar":50,"wr":42,"br":60},
    "rows": [
        ("Diagonal Corners",6666,50,86.2,8.2,5.6,80.6,50,42,63,"a1 e5"),
        ("Adjacent Corners",6667,50,86.7,8.1,5.2,81.5,50,42,58,"a1 a5"),
    ],
}

# Standard, played by the TILTAK engine (independent cross-check of the standard imbalance).
standard_5s_tiltak = {
    "title": "Standard Self-Play Balance  (Tiltak)",
    "subtitle": "5×5 board  ·  20,000 self-play games  ·  15,000 nodes per move  ·  no komi  ·  Tiltak engine",
    "overall": {"w":86.2,"d":6.7,"b":7.1,"gap":79.1,"ar":33,"wr":73,"br":59},
    "rows": [
        ("Diagonal Corners",6666,33.3,85.1,7.1,7.8,77.3,33,72,59,"a1 e5"),
        ("Adjacent Corners",6667,33.3,86.7,6.3,7.0,79.8,33,74,57,"a1 a5"),
        ("Hug",6667,33.3,86.8,6.7,6.5,80.3,33,73,61,"a1 b1"),
    ],
}

# ── 5×5 deeper-search (80k nodes) set, with engine badges ───────────────────
black_stack_5s_80k = {
    "title": "Double Black Stack Self-Play Balance", "engine": "Topaz bs-v3",
    "subtitle": "5×5 board  ·  20,000 self-play games  ·  80,000 nodes per move",
    "overall": {"w":36.4,"d":28.4,"b":35.1,"gap":1.3,"ar":20,"wr":39,"br":40},
    "rows": [
        ("Diagonal Corners",4000,20,45.4,27.8,26.8,18.5,20,38,38,"2a1 e5"),
        ("Adjacent Corners",4000,20,41.8,28.6,29.6,12.1,20,39,42,"2a1 a5"),
        ("Hug",4000,20,32.5,29.4,38.1,-5.6,20,40,40,"2a1 b1"),
        ("Gap Hug",4000,20,40.2,29.0,30.8,9.5,20,37,40,"2a1 c1"),
        ("Center Stack",4000,20,22.2,27.4,50.4,-28.1,20,40,39,"2c3 a1"),
    ],
}
# Sans center-stack (the −28.1% outlier); overall over the other 4 (16,000 games): W 6396 / D 4589 / B 5015.
black_stack_5s_sans_80k = {
    "title": "Double Black Stack Self-Play Balance", "engine": "Topaz bs-v3",
    "subtitle": "5×5 board  ·  16,000 self-play games  ·  80,000 nodes per move  ·  excludes center-stack opening",
    "overall": {"w":40.0,"d":28.7,"b":31.3,"gap":8.6,"ar":25,"wr":38,"br":40},
    "rows": [
        ("Diagonal Corners",4000,25,45.4,27.8,26.8,18.5,25,38,38,"2a1 e5"),
        ("Adjacent Corners",4000,25,41.8,28.6,29.6,12.1,25,39,42,"2a1 a5"),
        ("Hug",4000,25,32.5,29.4,38.1,-5.6,25,40,40,"2a1 b1"),
        ("Gap Hug",4000,25,40.2,29.0,30.8,9.5,25,37,40,"2a1 c1"),
    ],
}
standard_5s_80k = {
    "title": "Standard Self-Play Balance", "engine": "Topaz bs-v3",
    "subtitle": "5×5 board  ·  20,000 self-play games  ·  80,000 nodes per move  ·  no komi",
    "overall": {"w":93.4,"d":4.7,"b":1.9,"gap":91.5,"ar":33,"wr":39,"br":62},
    "rows": [
        ("Diagonal Corners",6666,33.3,92.3,5.4,2.3,90.0,33,40,64,"a1 e5"),
        ("Adjacent Corners",6667,33.3,92.5,5.4,2.2,90.3,33,39,59,"a1 a5"),
        ("Hug",6667,33.3,95.4,3.5,1.2,94.2,33,39,64,"a1 b1"),
    ],
}
standard_5s_tiltak_80k = {
    "title": "Standard Self-Play Balance", "engine": "Tiltak",
    "subtitle": "5×5 board  ·  20,000 self-play games  ·  80,000 nodes per move  ·  no komi",
    "overall": {"w":89.3,"d":7.1,"b":3.6,"gap":85.8,"ar":33,"wr":61,"br":50},
    "rows": [
        ("Diagonal Corners",6666,33.3,88.0,8.6,3.4,84.6,33,61,47,"a1 e5"),
        ("Adjacent Corners",6667,33.3,88.6,7.2,4.2,84.4,33,62,54,"a1 a5"),
        ("Hug",6667,33.3,91.4,5.5,3.0,88.4,33,62,47,"a1 b1"),
    ],
}

black_stack_5s_tiltak_80k = {
    "title": "Double Black Stack Self-Play Balance", "engine": "Tiltak",
    "subtitle": "5×5 board  ·  20,000 self-play games  ·  80,000 nodes per move",
    "overall": {"w":44.4,"d":21.9,"b":33.8,"gap":10.6,"ar":20,"wr":48,"br":48},
    "rows": [
        ("Diagonal Corners",4000,20,48.2,22.6,29.1,19.1,20,46,48,"2a1 e5"),
        ("Adjacent Corners",4000,20,50.2,20.4,29.5,20.7,20,47,47,"2a1 a5"),
        ("Hug",4000,20,44.8,22.8,32.5,12.3,20,53,47,"2a1 b1"),
        ("Gap Hug",4000,20,48.9,21.3,29.8,19.2,20,49,45,"2a1 c1"),
        ("Center Stack",4000,20,29.6,22.4,48.0,-18.4,20,47,50,"2c3 a1"),
    ],
}
# Sans center-stack (−18.4% outlier); overall over the other 4 (16,000): W 7685 / D 3482 / B 4833.
black_stack_5s_tiltak_sans_80k = {
    "title": "Double Black Stack Self-Play Balance", "engine": "Tiltak",
    "subtitle": "5×5 board  ·  16,000 self-play games  ·  80,000 nodes per move  ·  excludes center-stack opening",
    "overall": {"w":48.0,"d":21.8,"b":30.2,"gap":17.8,"ar":25,"wr":49,"br":47},
    "rows": [
        ("Diagonal Corners",4000,25,48.2,22.6,29.1,19.1,25,46,48,"2a1 e5"),
        ("Adjacent Corners",4000,25,50.2,20.4,29.5,20.7,25,47,47,"2a1 a5"),
        ("Hug",4000,25,44.8,22.8,32.5,12.3,25,53,47,"2a1 b1"),
        ("Gap Hug",4000,25,48.9,21.3,29.8,19.2,25,49,45,"2a1 c1"),
    ],
}

# ── 6×6 tiltak cross-checks (original distributions) ────────────────────────
black_stack_6s_tiltak = {
    "title": "Double Black Stack Self-Play Balance", "engine": "Tiltak",
    "subtitle": "6×6 board  ·  20,000 self-play games  ·  60,000 nodes per move",
    "overall": {"w":45.5,"d":14.6,"b":39.9,"gap":5.5,"ar":20,"wr":52,"br":52},
    "rows": [
        ("Diagonal Corners",4000,20,47.5,14.3,38.2,9.4,20,55,51,"2a1 f6"),
        ("Adjacent Corners",4000,20,46.7,14.8,38.4,8.3,20,53,52,"2a1 a6"),
        ("Hug",6000,30,47.0,14.8,38.2,8.7,30,49,51,"2a1 b1"),
        ("Gap Hug",4000,20,52.7,14.7,32.6,20.1,20,53,51,"2a1 c1"),
        ("Center Stack",2000,10,19.9,13.8,66.2,-46.3,10,52,55,"2c3 a1"),
    ],
}
# Sans center-stack (−46.3% outlier); overall over the other 4 (18,000): W 8695 / D 2642 / B 6663.
black_stack_6s_tiltak_sans = {
    "title": "Double Black Stack Self-Play Balance", "engine": "Tiltak",
    "subtitle": "6×6 board  ·  18,000 self-play games  ·  60,000 nodes per move  ·  excludes center-stack opening",
    "overall": {"w":48.3,"d":14.7,"b":37.0,"gap":11.3,"ar":22,"wr":52,"br":51},
    "rows": [
        ("Diagonal Corners",4000,22.2,47.5,14.3,38.2,9.4,22,55,51,"2a1 f6"),
        ("Adjacent Corners",4000,22.2,46.7,14.8,38.4,8.3,22,53,52,"2a1 a6"),
        ("Hug",6000,33.3,47.0,14.8,38.2,8.7,33,49,51,"2a1 b1"),
        ("Gap Hug",4000,22.2,52.7,14.7,32.6,20.1,22,53,51,"2a1 c1"),
    ],
}

two_komi_6s_tiltak = {
    "title": "2 Komi Self-Play Balance", "engine": "Tiltak",
    "subtitle": "6×6 board  ·  20,000 self-play games  ·  60,000 nodes per move",
    "overall": {"w":53.2,"d":17.1,"b":29.7,"gap":23.6,"ar":40,"wr":55,"br":45},
    "rows": [
        ("Diagonal Corners",8000,40,51.5,16.7,31.9,19.6,40,54,46,"a1 f6"),
        ("Adjacent Corners",8000,40,51.0,18.0,31.0,20.1,40,56,45,"a1 a6"),
        ("Hug",4000,20,61.2,16.1,22.7,38.5,20,54,43,"a1 b1"),
    ],
}
# Sans hug (+38.5% outlier); overall over diagonal + adjacent (16,000): W 8199 / D 2774 / B 5027.
two_komi_6s_tiltak_sans = {
    "title": "2 Komi Self-Play Balance", "engine": "Tiltak",
    "subtitle": "6×6 board  ·  16,000 self-play games  ·  60,000 nodes per move  ·  excludes hug opening",
    "overall": {"w":51.2,"d":17.3,"b":31.4,"gap":19.8,"ar":50,"wr":55,"br":45},
    "rows": [
        ("Diagonal Corners",8000,50,51.5,16.7,31.9,19.6,50,54,46,"a1 f6"),
        ("Adjacent Corners",8000,50,51.0,18.0,31.0,20.1,50,56,45,"a1 a6"),
    ],
}

# ── 4×4 ────────────────────────────────────────────────────────────────────
# PLACEHOLDERS: the per-archetype game COUNTS, SHARES, and example PTN below are exact (the 4x4
# opening book is exhaustively enumerated: 207 black-stack / 146 standard unique D4 positions, with
# disjoint round-robin buckets diagonal/adjacent/hug/gap-hug = 39/52/56/60 and standard
# diagonal/adjacent/hug = 35/49/62). The win/draw/black/gap/road numbers (wp,dp,bp,gap,wr,br and the
# `overall` block) are ZEROED — fill them from `balance_results/balance_{blackstack,standard,
# standard_tiltak}.txt` produced by run-balance4.sh, then uncomment the make_sheet calls below.
# Both variants are measured with the SAME black-stack net at komi 0 (standard 4x4 is too imbalanced
# to train a native net — same finding as 5x5). `engine` badge = the net / engine used.
black_stack_4s = {
    "title": "Double Black Stack Self-Play Balance", "engine": "Topaz bs-v1",
    "subtitle": "4×4 board  ·  207 self-play games (full opening book)  ·  60,000 nodes per move",
    "overall": {"w":0,"d":0,"b":0,"gap":0,"ar":0,"wr":0,"br":0},
    "rows": [
        ("Diagonal Corners",39,18.8,0,0,0,0,19,0,0,"2a1 d4"),
        ("Adjacent Corners",52,25.1,0,0,0,0,25,0,0,"2a1 a4"),
        ("Hug",56,27.1,0,0,0,0,27,0,0,"2a1 b1"),
        ("Gap Hug",60,29.0,0,0,0,0,29,0,0,"2a1 c1"),
    ],
}
standard_4s = {
    "title": "Standard Self-Play Balance", "engine": "Topaz bs-v1",
    "subtitle": "4×4 board  ·  146 self-play games (full opening book)  ·  60,000 nodes per move  ·  no komi",
    "overall": {"w":0,"d":0,"b":0,"gap":0,"ar":0,"wr":0,"br":0},
    "rows": [
        ("Diagonal Corners",35,24.0,0,0,0,0,24,0,0,"a1 d4"),
        ("Adjacent Corners",49,33.6,0,0,0,0,34,0,0,"a1 a4"),
        ("Hug",62,42.5,0,0,0,0,42,0,0,"a1 b1"),
    ],
}
standard_4s_tiltak = {
    "title": "Standard Self-Play Balance", "engine": "Tiltak",
    "subtitle": "4×4 board  ·  146 self-play games (full opening book)  ·  60,000 nodes per move  ·  no komi  ·  Tiltak engine",
    "overall": {"w":0,"d":0,"b":0,"gap":0,"ar":0,"wr":0,"br":0},
    "rows": [
        ("Diagonal Corners",35,24.0,0,0,0,0,24,0,0,"a1 d4"),
        ("Adjacent Corners",49,33.6,0,0,0,0,34,0,0,"a1 a4"),
        ("Hug",62,42.5,0,0,0,0,42,0,0,"a1 b1"),
    ],
}

D = r"C:\Users\lance\Desktop\Tak\Bots\topaz-tak"
# 6×6 sheets (already generated; outputs now under 6x6/results/). Uncomment to regenerate.
# make_sheet(black_stack,      D + r"\6x6\results\black-stack-results.png")
# make_sheet(two_komi,         D + r"\6x6\results\2-komi-results.png")
# make_sheet(black_stack_sans, D + r"\6x6\results\black-stack-results-sans-center-stack.png")
# make_sheet(two_komi_sans,    D + r"\6x6\results\2-komi-results-sans-hug.png")
# 5×5 sheets
make_sheet(black_stack_5s,      D + r"\5x5\results\5s-black-stack-results.png")
make_sheet(black_stack_5s_sans, D + r"\5x5\results\5s-black-stack-results-sans-center-stack.png")
make_sheet(standard_5s,         D + r"\5x5\results\5s-standard-results.png")
make_sheet(standard_5s_sans,    D + r"\5x5\results\5s-standard-results-sans-hug.png")
make_sheet(standard_5s_tiltak,  D + r"\5x5\results\5s-standard-tiltak-results.png")
make_sheet(black_stack_5s_80k,      D + r"\5x5\results\5s-black-stack-80k-results.png")
make_sheet(black_stack_5s_sans_80k, D + r"\5x5\results\5s-black-stack-80k-results-sans-center-stack.png")
make_sheet(standard_5s_80k,         D + r"\5x5\results\5s-standard-80k-results.png")
make_sheet(standard_5s_tiltak_80k,  D + r"\5x5\results\5s-standard-tiltak-80k-results.png")
make_sheet(black_stack_5s_tiltak_80k,      D + r"\5x5\results\5s-black-stack-tiltak-80k-results.png")
make_sheet(black_stack_5s_tiltak_sans_80k, D + r"\5x5\results\5s-black-stack-tiltak-80k-results-sans-center-stack.png")
make_sheet(black_stack_6s_tiltak,      D + r"\6x6\results\6s-black-stack-tiltak-results.png")
make_sheet(black_stack_6s_tiltak_sans, D + r"\6x6\results\6s-black-stack-tiltak-results-sans-center-stack.png")
make_sheet(two_komi_6s_tiltak,      D + r"\6x6\results\6s-2komi-tiltak-results.png")
make_sheet(two_komi_6s_tiltak_sans, D + r"\6x6\results\6s-2komi-tiltak-results-sans-hug.png")
# 4×4 sheets — fill the placeholder win/draw/gap numbers from balance_results/*.txt, create
# 4x4/results/, then uncomment. (Plain PNGs only — no combined results.html.)
# make_sheet(black_stack_4s,     D + r"\4x4\results\4s-black-stack-results.png")
# make_sheet(standard_4s,        D + r"\4x4\results\4s-standard-results.png")
# make_sheet(standard_4s_tiltak, D + r"\4x4\results\4s-standard-tiltak-results.png")
