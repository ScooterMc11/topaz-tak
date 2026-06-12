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
D = r"C:\Users\lance\Desktop\Tak\Bots\topaz-tak"
make_sheet(black_stack,      D + r"\black-stack-results.png")
make_sheet(two_komi,         D + r"\2-komi-results.png")
make_sheet(black_stack_sans, D + r"\black-stack-results-sans-center-stack.png")
make_sheet(two_komi_sans,    D + r"\2-komi-results-sans-hug.png")
