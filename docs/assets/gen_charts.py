#!/usr/bin/env python3
"""Generate dependency-free SVG charts for the ethresear.ch IR-trace draft.

All figures are derived from the benchmark numbers in README.md and
docs/ir-trace-benchmark.md. Run from the repo root:

    python3 docs/assets/gen_charts.py

Outputs four SVGs into docs/assets/.
"""

import math
import os
from xml.sax.saxutils import escape

OUT_DIR = os.path.dirname(os.path.abspath(__file__))

FONT = "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif"
INK = "#1f2933"
MUTED = "#6b7280"
GRID = "#e5e7eb"
LEAN = "#6b8afd"
RUST = "#3ec9a7"
OK = "#3ec9a7"
OVER = "#ef5d5d"
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
    """Log-scale horizontal bars: sum trace vs ETH2 trace, with the input limit."""
    W, H = 760, 320
    px, py, pw = 150, 96, 540
    lo, hi = 3.0, 10.0  # log10 bytes: 1 KB .. 10 GB

    def lx(v):
        return px + (math.log10(v) - lo) / (hi - lo) * pw

    body = [
        text(40, 34, "The trace-size wall (serialized bincode, log scale)", size=17, weight="600"),
        text(40, 54, "Sum example fits easily; the ETH2 STF trace blows past the input limit.", size=12, fill=MUTED),
    ]
    # x gridlines: 1KB,1MB,1GB,10GB
    for expo, lab in [(3, "1 KB"), (6, "1 MB"), (9, "1 GB"), (10, "10 GB")]:
        gx = lx(10 ** expo)
        body.append(line(gx, py - 10, gx, py + 120, w=1))
        body.append(text(gx, py + 138, lab, size=11, anchor="middle", fill=MUTED))
    # bars
    bars = [("sum example", 3843, "3.8 KB", OK), ("ETH2 STF", 8.14e9, "8.14 GB", OVER)]
    bh = 40
    for i, (name, val, lab, color) in enumerate(bars):
        by = py + i * 64
        body.append(text(px - 16, by + bh / 2 + 4, name, size=13, anchor="end"))
        body.append(rect(px, by, lx(val) - px, bh, color))
        body.append(text(lx(val) + 8, by + bh / 2 + 4, lab, size=12, weight="600", fill=color))
    # limit line at 2^32 bytes (~4 GB)
    limit = 2 ** 32
    lxp = lx(limit)
    body.append(line(lxp, py - 14, lxp, py + 124, stroke=LIMIT, w=2, dash="5,4"))
    body.append(text(lxp, py - 22, "~4 GB zkVM input limit", size=12, anchor="middle", fill=LIMIT, weight="600"))
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
    """Vertical bars: per-metric growth ratio from N=10 to N=100 (1.0x = no growth)."""
    W, H = 760, 400
    px, py, pw, ph = 80, 86, 640, 250
    axis_max = 2.0
    metrics = [
        ("Total steps", 1.36),
        ("Wall time", 1.45),
        ("Value table", 1.36),
        ("PrimResult", 1.57),
        ("Output size", 1.17),
    ]
    body = [
        text(px, 34, "Scaling: how each metric grows from N=10 to N=100", size=17, weight="600"),
        text(px, 54, "Validators grew 10x; recorded work grows far more slowly.", size=12, fill=MUTED),
    ]
    for i in range(5):
        v = i * 0.5
        gy = py + ph - v / axis_max * ph
        body.append(line(px, gy, px + pw, gy))
        body.append(text(px - 10, gy + 4, f"{v:.1f}x", size=11, anchor="end", fill=MUTED))
    # 1.0x reference (no-growth) line
    ry = py + ph - 1.0 / axis_max * ph
    body.append(line(px, ry, px + pw, ry, stroke=MUTED, w=1.4, dash="5,4"))
    body.append(text(px + pw, ry - 6, "1.0x (no growth)", size=11, anchor="end", fill=MUTED))
    n = len(metrics)
    slot = pw / n
    bw = 64
    for i, (name, ratio) in enumerate(metrics):
        bx = px + i * slot + (slot - bw) / 2
        bh = ratio / axis_max * ph
        by = py + ph - bh
        body.append(rect(bx, by, bw, bh, BAR))
        body.append(text(bx + bw / 2, by - 8, f"{ratio}x", size=12, anchor="middle", weight="600"))
        body.append(text(bx + bw / 2, py + ph + 22, name, size=12, anchor="middle"))
    body.append(line(px, py + ph, px + pw, py + ph, stroke=MUTED, w=1.2))
    write("bench-scaling.svg", svg(W, H, "".join(body)))


if __name__ == "__main__":
    chart_zkvm_cycles()
    chart_trace_size_wall()
    chart_step_composition()
    chart_scaling()
