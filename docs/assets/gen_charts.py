#!/usr/bin/env python3
"""Generate dependency-free SVG charts for the ethresear.ch IR-trace draft.

All figures are derived from the benchmark numbers in README.md and
docs/ir-trace-benchmark.md. Run from the repo root:

    python3 docs/assets/gen_charts.py

Outputs four SVGs into docs/assets/.
"""

import os
from xml.sax.saxutils import escape

OUT_DIR = os.path.dirname(os.path.abspath(__file__))

FONT = "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif"
INK = "#1f2933"
MUTED = "#6b7280"
GRID = "#e5e7eb"
LEAN = "#6b8afd"
RUST = "#3ec9a7"
BAR = "#6b8afd"
LIMIT = "#ef5d5d"


def text(x, y, s, size=13, anchor="start", fill=INK, weight="normal"):
    return (
        f'<text x="{x:.1f}" y="{y:.1f}" font-family="{FONT}" '
        f'font-size="{size}" text-anchor="{anchor}" fill="{fill}" '
        f'font-weight="{weight}">{escape(str(s))}</text>'
    )


def rect(x, y, w, h, fill, rx=2):
    return f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" rx="{rx}" fill="{fill}"/>'


def line(x1, y1, x2, y2, stroke=GRID, w=1, dash=None):
    d = f' stroke-dasharray="{dash}"' if dash else ""
    return f'<line x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" stroke="{stroke}" stroke-width="{w}"{d}/>'


def svg(width, height, body):
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'viewBox="0 0 {width} {height}" role="img">'
        f'{rect(0, 0, width, height, "#ffffff", rx=0)}{body}</svg>'
    )


def write(name, content):
    path = os.path.join(OUT_DIR, name)
    with open(path, "w", encoding="utf-8") as f:
        f.write(content)
    print(f"wrote {path}")


def legend(x, y, items):
    out = []
    for label, color in items:
        out.append(rect(x, y - 9, 12, 12, color))
        out.append(text(x + 18, y + 1, label, size=12, fill=MUTED))
        x += 18 + 7.2 * len(label) + 22
    return "".join(out)


def chart_zkvm_cycles():
    """Grouped bars: compiled Lean vs Rust zkVM cycles at N=10 and N=100."""
    W, H = 760, 430
    px, py, pw, ph = 80, 78, 640, 280
    axis_max = 40_000_000
    groups = [("N=10", 26_148_291, 12_491_509), ("N=100", 35_281_299, 14_446_747)]
    body = [
        text(px, 34, "zkVM cycles: compiled Lean vs Rust", size=17, weight="600"),
        text(px, 54, "Lower is better. IR Trace (zkVM verify) is N/A — blocked by trace size.", size=12, fill=MUTED),
        legend(px + 330, 44, [("Lean (compiled)", LEAN), ("Rust (compiled)", RUST)]),
    ]
    # gridlines + y labels (0..40M every 10M)
    for i in range(5):
        v = i * 10_000_000
        gy = py + ph - v / axis_max * ph
        body.append(line(px, gy, px + pw, gy))
        body.append(text(px - 10, gy + 4, f"{v // 1_000_000}M", size=11, anchor="end", fill=MUTED))
    gw = pw / 2
    bw, gap = 78, 26
    for gi, (gname, lean, rust) in enumerate(groups):
        gleft = px + gi * gw
        start = gleft + (gw - (bw * 2 + gap)) / 2
        for bi, (val, color) in enumerate([(lean, LEAN), (rust, RUST)]):
            bx = start + bi * (bw + gap)
            bh = val / axis_max * ph
            by = py + ph - bh
            body.append(rect(bx, by, bw, bh, color))
            body.append(text(bx + bw / 2, by - 8, f"{val/1e6:.1f}M", size=12, anchor="middle", weight="600"))
        body.append(text(gleft + gw / 2, py + ph + 24, gname, size=13, anchor="middle"))
    body.append(line(px, py + ph, px + pw, py + ph, stroke=MUTED, w=1.2))
    write("bench-zkvm-cycles.svg", svg(W, H, "".join(body)))


def chart_trace_size_wall():
    """Stacked fixed + per-validator decomposition of the trace, vs the input limit."""
    W, H = 760, 340
    px, py, pw = 150, 100, 520
    gmax = 12.0  # GB
    FIXED, PROP = BAR, "#f0a500"
    fixed = 7.81
    rows = [("V=1", 0.0327, "7.85"), ("V=10", 0.327, "8.14"), ("V=100", 3.27, "11.08")]

    def wd(g):
        return g / gmax * pw

    body = [
        text(40, 34, "Even one validator's trace busts the input limit", size=17, weight="600"),
        text(40, 54, "The fixed ~7.81 GB structural term alone exceeds the ~4 GB limit at every V.", size=12, fill=MUTED),
        legend(px, 78, [("fixed (structural)", FIXED), ("per-validator", PROP)]),
    ]
    # x gridlines (0..12 GB, every 3)
    for g in range(0, 13, 3):
        gx = px + wd(g)
        body.append(line(gx, py - 6, gx, py + 206, w=1))
        body.append(text(gx, py + 224, f"{g} GB", size=11, anchor="middle", fill=MUTED))
    bh, gap = 44, 22
    for i, (name, prop, total_lab) in enumerate(rows):
        by = py + i * (bh + gap)
        body.append(text(px - 16, by + bh / 2 + 4, name, size=13, anchor="end"))
        body.append(rect(px, by, wd(fixed), bh, FIXED))
        body.append(rect(px + wd(fixed), by, wd(prop), bh, PROP))
        body.append(text(px + wd(fixed + prop) + 8, by + bh / 2 + 4, f"{total_lab} GB", size=12, weight="600", fill=INK))
    # 4 GB input limit
    lx = px + wd(4.0)
    body.append(line(lx, py - 12, lx, py + 206, stroke=LIMIT, w=2, dash="5,4"))
    body.append(text(lx, py - 20, "~4 GB input limit", size=12, anchor="middle", fill=LIMIT, weight="600"))
    write("bench-trace-size-wall.svg", svg(W, H, "".join(body)))


def chart_step_composition():
    """Horizontal bars: IR-trace step-type breakdown at N=10."""
    W, H = 760, 360
    px, py, pw = 150, 90, 470
    steps = [
        ("PrimResult", 100_490, 42.2),
        ("Call", 51_128, 21.5),
        ("ProjResult", 39_701, 16.7),
        ("Branch", 27_111, 11.4),
        ("CtorCreate", 10_865, 4.6),
        ("SetResult", 8_754, 3.7),
    ]
    maxv = steps[0][1]
    body = [
        text(40, 34, "IR-trace step composition (ETH2, N=10)", size=17, weight="600"),
        text(40, 54, "238,049 total steps recorded by the host interpreter.", size=12, fill=MUTED),
    ]
    bh, gap = 30, 14
    for i, (name, cnt, pct) in enumerate(steps):
        by = py + i * (bh + gap)
        body.append(text(px - 16, by + bh / 2 + 4, name, size=13, anchor="end"))
        bwl = cnt / maxv * pw
        body.append(rect(px, by, bwl, bh, BAR))
        body.append(text(px + bwl + 8, by + bh / 2 + 4, f"{cnt:,} ({pct}%)", size=12, anchor="start", fill=INK))
    write("bench-step-composition.svg", svg(W, H, "".join(body)))


def chart_scaling():
    """Affine fit: serialized trace size (GB) vs validator count."""
    W, H = 760, 430
    px, py, pw, ph = 84, 92, 596, 268
    vmax, gmax = 105.0, 12.0  # validators, GB
    fixed, slope_gb = 7.81, 0.0327  # GB, GB/validator

    def X(v):
        return px + v / vmax * pw

    def Y(g):
        return py + ph - g / gmax * ph

    body = [
        text(px, 34, "Serialized trace size is affine in validator count", size=17, weight="600"),
        text(px, 54, "~7.81 GB fixed + ~32.7 MB/validator. The whole line sits above the ~4 GB input limit.", size=12, fill=MUTED),
    ]
    # y gridlines (0..12 GB, every 3)
    for g in range(0, 13, 3):
        gy = Y(g)
        body.append(line(px, gy, px + pw, gy))
        body.append(text(px - 10, gy + 4, f"{g} GB", size=11, anchor="end", fill=MUTED))
    # x ticks
    for v in (0, 25, 50, 75, 100):
        body.append(text(X(v), py + ph + 22, str(v), size=11, anchor="middle", fill=MUTED))
    body.append(text(px + pw / 2, py + ph + 44, "validators (V)", size=12, anchor="middle", fill=MUTED))
    # 4 GB input limit
    ly = Y(4.0)
    body.append(line(px, ly, px + pw, ly, stroke=LIMIT, w=2, dash="5,4"))
    body.append(text(px + pw, ly - 7, "~4 GB input limit", size=12, anchor="end", fill=LIMIT, weight="600"))
    # fitted line V=0..100
    body.append(line(X(0), Y(fixed), X(100), Y(fixed + slope_gb * 100), stroke=BAR, w=2.5))
    body.append(text(X(0) + 8, Y(fixed) - 18, "7.81 GB fixed (V=0)", size=11, fill=BAR, weight="600"))
    body.append(text(X(100), Y(fixed + slope_gb * 100) - 10, "≈11.1 GB (est.)", size=11, anchor="end", fill=MUTED))
    # measured points
    for v, g in [(1, 7.85), (2, 7.88), (3, 7.91), (10, 8.14)]:
        body.append(f'<circle cx="{X(v):.1f}" cy="{Y(g):.1f}" r="4.5" fill="{INK}"/>')
    body.append(text(X(10) + 12, Y(8.14) + 26, "measured V=1,2,3,10", size=11, anchor="start", fill=INK))
    # axes
    body.append(line(px, py, px, py + ph, stroke=MUTED, w=1.2))
    body.append(line(px, py + ph, px + pw, py + ph, stroke=MUTED, w=1.2))
    write("bench-scaling.svg", svg(W, H, "".join(body)))


if __name__ == "__main__":
    chart_zkvm_cycles()
    chart_trace_size_wall()
    chart_step_composition()
    chart_scaling()
