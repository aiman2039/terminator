# Renderer comparison — 2026-10-07

The `terminator/wgpu` feature selects eframe's Wgpu renderer. Glow remains the
default. Both native release builds completed all six comparison trials.

Wgpu used **23.3% less GUI CPU time during terminal output** on
this Mac. Idle CPU use was close. Output memory use was about 7 MiB higher.
These results support a Wgpu trial on macOS; they do not establish a benefit on
other machines or platforms.

| Median of three trials | Glow | Wgpu / Metal |
| --- | ---: | ---: |
| Idle GUI CPU, one core = 100% | 2.24% | 2.13% |
| Output GUI CPU, one core = 100% | 12.15% | 9.32% |
| Idle GUI peak RSS, MiB | 208.61 | 205.81 |
| Output GUI peak RSS, MiB | 211.78 | 218.69 |
| GUI executable, MiB | 41.72 | 47.22 |

## Method

Apple M5 Max, Mac17,6, 18 CPU cores, macOS 27.0 (26A428), arm64.
Source base: `236e263` plus the local feature and fixture changes.
App 0.92.0; eframe/egui-wgpu 0.36.1; egui 0.36.2; wgpu 30.0.1.

Both builds used the release profile, workspace binaries and examples, and
`terminator/test-support`. The candidate also enabled `terminator/wgpu`.
The daemon, hook, layout generator and measurement driver were identical.
No compiler or test build ran during measurement.

Each trial used new isolated state and six visible terminal panes at 1440 × 900,
scale 1. The windows stayed inactive and passed mouse input through. Idle shells
used the fixture's plain shell configuration. Output shells printed the same
line approximately every 33 ms. Shell scheduling can reduce the actual rate.
Both output captures showed all six panes with output.

Each GUI warmed up for five seconds. Measurement then ran for at least
15 seconds. `ps -o time= -o rss=` sampled the GUI every 500 ms.
CPU percentage is the change in cumulative GUI CPU milliseconds divided by
elapsed seconds and by 10. RSS is the largest sample during measurement.
The screenshot driver stopped its forced redraw polling after three seconds.
Captures occurred after the measurement interval.

Wgpu ran first, then Glow. Trial order was not randomized or reversed.
Only this machine, fixed pane layout and short workload were measured.
No GPU time, power, frame rate, input delay, startup time, browser panes,
sleep recovery, Linux or Windows results are available.

An initial covered-window Wgpu capture timed out. Its results and the earlier
covered-window Glow results are excluded. The final visible-window runs each
produced six valid native captures. Wgpu logs identify the Apple M5 Max adapter
and Metal backend. Upstream also documents foreground capture requirements:
[egui inspection guidance](https://github.com/emilk/egui/blob/main/crates/egui_inspection/README.md).

## Raw evidence

| Renderer | Workload | Trial | GUI CPU | GUI peak RSS, MiB |
| --- | --- | ---: | ---: | ---: |
| Glow | idle | 1 | 2.15% | 208.61 |
| Glow | idle | 2 | 2.46% | 202.66 |
| Glow | idle | 3 | 2.24% | 210.52 |
| Glow | output | 1 | 11.46% | 211.78 |
| Glow | output | 2 | 12.15% | 211.17 |
| Glow | output | 3 | 12.37% | 211.91 |
| Wgpu | idle | 1 | 2.08% | 206.08 |
| Wgpu | idle | 2 | 2.13% | 200.16 |
| Wgpu | idle | 3 | 2.35% | 205.81 |
| Wgpu | output | 1 | 9.32% | 218.69 |
| Wgpu | output | 2 | 9.39% | 219.17 |
| Wgpu | output | 3 | 9.26% | 218.67 |

Local raw reports and captures:
[Glow](../target/validation/renderer-glow-visible/renderer-perf/renderer-perf.json),
[Wgpu](../target/validation/renderer-wgpu-visible/renderer-perf/renderer-perf.json).
Repeat commands are in [scripts/README.md](../scripts/README.md#renderer-comparison).

GUI SHA-256:

- Glow: `adbbed713e4f9cf710fb8ca6fa1f0bf7b763519c3d360b9f468bc036f202859e`
- Wgpu: `335148d69cc004c925fc8a6a35f3d0fac545e3f91d14e79b98d704f5aacefaed`

## Checks

Both release configurations built with the lockfile. Focused strict Clippy checks
passed for the app and measurement driver. The CPU-time parser test passed.
Formatting and diff whitespace checks passed. Native idle and output runs passed
for both renderers; representative captures were inspected.

The installed app and its live daemon were not replaced. Existing workflow edits
were preserved.
