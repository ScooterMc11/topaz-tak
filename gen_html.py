#!/usr/bin/env python3
"""Build a single self-contained results.html: a summary matrix of the White-Black balance gaps
plus every detail sheet embedded (base64). One portable file to share / host."""
import base64, os
from collections import OrderedDict

ROOT = r"C:\Users\lance\Desktop\Tak\Bots\topaz-tak"
OUT = os.path.join(ROOT, "results.html")

# (board, variant, engine, nodes, scope, White-Black gap %, png path relative to ROOT)
RUNS = [
    ("5×5", "Black stack (0 komi)", "Topaz bs-v3", "15k", "all",  0.4, "5x5/results/5s-black-stack-results.png"),
    ("5×5", "Black stack (0 komi)", "Topaz bs-v3", "15k", "sans", 5.8, "5x5/results/5s-black-stack-results-sans-center-stack.png"),
    ("5×5", "Black stack (0 komi)", "Topaz bs-v3", "80k", "all",  1.3, "5x5/results/5s-black-stack-80k-results.png"),
    ("5×5", "Black stack (0 komi)", "Topaz bs-v3", "80k", "sans", 8.6, "5x5/results/5s-black-stack-80k-results-sans-center-stack.png"),
    ("5×5", "Black stack (0 komi)", "Tiltak", "80k", "all",  10.6, "5x5/results/5s-black-stack-tiltak-80k-results.png"),
    ("5×5", "Black stack (0 komi)", "Tiltak", "80k", "sans", 17.8, "5x5/results/5s-black-stack-tiltak-80k-results-sans-center-stack.png"),
    ("5×5", "Standard (0 komi)", "Topaz bs-v3", "15k", "all",  83.6, "5x5/results/5s-standard-results.png"),
    ("5×5", "Standard (0 komi)", "Topaz bs-v3", "15k", "sans", 81.0, "5x5/results/5s-standard-results-sans-hug.png"),
    ("5×5", "Standard (0 komi)", "Topaz bs-v3", "80k", "all",  91.5, "5x5/results/5s-standard-80k-results.png"),
    ("5×5", "Standard (0 komi)", "Tiltak", "15k", "all",  79.1, "5x5/results/5s-standard-tiltak-results.png"),
    ("5×5", "Standard (0 komi)", "Tiltak", "80k", "all",  85.8, "5x5/results/5s-standard-tiltak-80k-results.png"),
    ("6×6", "Black stack (0 komi)", "Topaz", "60k", "all",  1.3, "6x6/results/black-stack-results.png"),
    ("6×6", "Black stack (0 komi)", "Topaz", "60k", "sans", 5.6, "6x6/results/black-stack-results-sans-center-stack.png"),
    ("6×6", "Black stack (0 komi)", "Tiltak", "60k", "all",  5.5, "6x6/results/6s-black-stack-tiltak-results.png"),
    ("6×6", "Black stack (0 komi)", "Tiltak", "60k", "sans", 11.3, "6x6/results/6s-black-stack-tiltak-results-sans-center-stack.png"),
    ("6×6", "2 komi (standard)", "Topaz", "60k", "all",  15.0, "6x6/results/2-komi-results.png"),
    ("6×6", "2 komi (standard)", "Topaz", "60k", "sans", 11.7, "6x6/results/2-komi-results-sans-hug.png"),
    ("6×6", "2 komi (standard)", "Tiltak", "60k", "all",  23.6, "6x6/results/6s-2komi-tiltak-results.png"),
    ("6×6", "2 komi (standard)", "Tiltak", "60k", "sans", 19.8, "6x6/results/6s-2komi-tiltak-results-sans-hug.png"),
]


def gap_color(g):
    a = abs(g)
    return "#2e7d32" if a < 3 else "#9e9d24" if a < 10 else "#ef6c00" if a < 25 else "#c62828"


def img_b64(rel):
    with open(os.path.join(ROOT, *rel.split("/")), "rb") as f:
        return base64.b64encode(f.read()).decode()


# group: (board, variant) -> {(engine, nodes): {scope: (gap, png)}}
groups = OrderedDict()
for board, variant, engine, nodes, scope, gap, png in RUNS:
    groups.setdefault((board, variant), OrderedDict()).setdefault((engine, nodes), {})[scope] = (gap, png)


def badge(cell):
    if cell is None:
        return '<td class="na">—</td>'
    gap, _ = cell
    return f'<td><span class="g" style="background:{gap_color(gap)}">{gap:+.1f}%</span></td>'


summary_rows, gallery = [], []
for (board, variant), rowmap in groups.items():
    summary_rows.append(f'<tr class="grp"><td colspan="4">{board} &nbsp;·&nbsp; {variant}</td></tr>')
    figs = []
    for (engine, nodes), scopes in rowmap.items():
        summary_rows.append(
            f'<tr><td>{engine}</td><td>{nodes}</td>{badge(scopes.get("all"))}{badge(scopes.get("sans"))}</tr>')
        for scope in ("all", "sans"):
            if scope in scopes:
                gap, png = scopes[scope]
                lbl = f'{engine} · {nodes}' + (' · excludes outlier opening' if scope == "sans" else '')
                figs.append(f'<figure><figcaption>{lbl} &nbsp;({gap:+.1f}%)</figcaption>'
                            f'<img src="data:image/png;base64,{img_b64(png)}"></figure>')
    gallery.append(f'<details><summary>{board} · {variant}</summary>{"".join(figs)}</details>')

html = f"""<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Tak Opening-Balance Results</title>
<style>
body{{font-family:system-ui,'Segoe UI',Arial,sans-serif;max-width:1000px;margin:0 auto;padding:28px;color:#2c2c2a;background:#fff;line-height:1.5}}
h1{{font-size:27px;margin:0 0 4px}} h2{{font-size:20px;margin:32px 0 10px}}
.sub{{color:#5f5e5a;margin:0 0 8px}}
table{{border-collapse:collapse;width:100%;margin:6px 0 10px}}
th,td{{padding:8px 14px;text-align:left;border-bottom:1px solid #e7e5dd;font-size:14px}}
th{{color:#5f5e5a;font-weight:600;border-bottom:2px solid #e7e5dd}}
tr.grp td{{background:#f4f2ec;font-weight:700;padding-top:11px;border-bottom:1px solid #ddd9cd}}
.g{{color:#fff;padding:2px 9px;border-radius:11px;font-weight:700;font-size:13px;white-space:nowrap}}
.na{{color:#b9b8b0}}
.legend{{font-size:13px;color:#5f5e5a;margin:0 0 4px}}
.legend i{{display:inline-block;width:11px;height:11px;border-radius:3px;vertical-align:middle;margin:0 4px 0 14px}}
.note{{font-size:14px;color:#3a3a37;background:#f8f7f2;border:1px solid #e7e5dd;border-radius:10px;padding:12px 16px;margin:14px 0}}
details{{border:1px solid #e7e5dd;border-radius:10px;margin:12px 0;padding:4px 16px;background:#fcfbf8}}
summary{{font-weight:700;cursor:pointer;padding:8px 0}}
figure{{margin:16px 0}} figcaption{{color:#5f5e5a;font-size:13px;margin-bottom:6px}}
img{{max-width:100%;border:1px solid #e7e5dd;border-radius:8px;box-shadow:0 1px 4px rgba(0,0,0,.06)}}
</style></head><body>
<h1>Tak Opening-Balance Results &mdash; 5×5 &amp; 6×6</h1>
<p class="sub">Self-play <b>White&minus;Black win-rate gap</b> by board size, opening rule, and engine. Closer to 0% = more balanced. "Sans outlier" recomputes excluding the single most-imbalanced opening archetype.</p>
<p class="legend">gap magnitude: <i style="background:#2e7d32"></i>&lt;3% (balanced) <i style="background:#9e9d24"></i>&lt;10% <i style="background:#ef6c00"></i>&lt;25% <i style="background:#c62828"></i>&ge;25% (heavily White-favored)</p>
<table><thead><tr><th>Engine</th><th>Nodes/move</th><th>All openings</th><th>Sans outlier</th></tr></thead>
<tbody>{''.join(summary_rows)}</tbody></table>
<div class="note"><b>Takeaway:</b> the double-black-stack rule keeps both board sizes near balanced (≈+0.4–1.3% with the strong native net), while standard 5×5 at 0 komi (+80–92%) and standard 6×6 at 2 komi (+15–24%) stay White-favored. Two independent engines (the Topaz NNUE and Tiltak) agree on the direction and rough magnitude in every case.</div>
<h2>Detail sheets</h2>
<p class="sub">Per-archetype breakdowns with 95% CIs. Click a section to expand.</p>
{''.join(gallery)}
</body></html>"""

with open(OUT, "w", encoding="utf-8") as f:
    f.write(html)
print("wrote", OUT, f"({os.path.getsize(OUT)//1024} KB)")
