# xlr8

**GitHub in a box: instant local web viewer for any repo.**
One command, one ~5 MB binary. Commits, compare, search and PRs, served from your own machine. Runs on Windows, Linux, and even a phone (Termux).

```
xlr8 owner/repo
```

- **Repeat views in ~0.1 ms.** Results are cached in memory and on disk, so they survive restarts.
- **First compare 1.1–2.6× faster than `git diff`'s own working time** on 7 of 8 benchmark ranges (16 threads, Windows). Against git's *wall-clock* — which includes ~28 ms of process spawn that a running server never pays — the gap looks like 1.7–5.3×; the internal number is the honest one.
- **Git-accurate.** Exact totals on synthetic ranges, within 0.15% of additions and 0.65% of deletions on real repos, and identical rename pairs (29/29 on tokio).

![xlr8 commits view](assets/screenshot_commits.png)

<p align="center"><sub>tokio: 4,783 commits, browsed locally</sub></p>

<table>
  <tr>
    <td><img src="assets/screenshot_compare.png" alt="Compare view" width="100%"></td>
    <td><img src="assets/screenshot_diffs_preview.png" alt="Diff view" width="100%"></td>
  </tr>
  <tr>
    <td align="center"><sub>tokio, 881 files compared in ~46 ms</sub></td>
    <td align="center"><sub>GitHub-style diffs</sub></td>
  </tr>
  <tr>
    <td colspan="2"><img src="assets/screenshot_code_search.png" alt="Code search" width="100%"></td>
  </tr>
  <tr>
    <td colspan="2" align="center"><sub>Fuzzy path search and in-repo grep across the whole tree, with a blob viewer</sub></td>
  </tr>
</table>

## Install

**Download a prebuilt binary** from [Releases](https://github.com/Banshal-Yadav/xlr8/releases) (**Windows, Linux x86_64**), unzip / untar, and run `xlr8 owner/repo`.

**Or build from source:**

```bash
git clone https://github.com/Banshal-Yadav/xlr8.git
cd xlr8
cargo build --release
```

Binary: `target/release/xlr8` (~5 MB). Requires the Rust toolchain ([rustup](https://rustup.rs)).
Other platforms (macOS, ARM/Termux): build from source.

## Usage

```
xlr8 owner/repo --port 7782     # open one repo
xlr8 --serve --port 7782        # serve all previously opened repos
```

Opens `http://127.0.0.1:7782`. The repo is mirrored to `~/.cache/xlr8/mirrors/` and synced on request.
Flags: `--port <p>` (default 7782) · `--serve` (multi-repo) · `--no-open` (don't launch browser).

## What it does

- **Commits**: paginate, per-commit +/−, activity strip, author rank
- **Compare**: rename detection, edit-script diffs, windowed rendering
- **Search**: fuzzy path matching, in-repo grep, blob viewer
- **Pulls**: browse open PRs
- **Perf bar**: live latency sparkline, cache meter, RAM/CPU

## Performance

**Windows 11 · Ryzen 7 7735HS (8C/16T) · NVMe.** Every xlr8 figure here is a
**median of 3 independent harness passes** (cold compare swings up to 2× between
passes, so a single run is not a publishable number). `git` is
`git diff --numstat -M` on the same packed mirror: **wall** includes ~28 ms of
process spawn, **internal** is `GIT_TRACE_PERFORMANCE` (git's own work only).
Compare xlr8 against the internal column — that is the per-core number. xlr8
times are full HTTP round-trips with JSON. Cold = fresh process, all caches
wiped. Warm = 200 repeats over a keep-alive connection.

| Range                            | Files | git wall | git intern. | xlr8 cold      | cold, 1 thread | warm    |
| -------------------------------- | ----- | -------- | ----------- | -------------- | -------------- | ------- |
| ripgrep 14.1..15.1               | 91    | 47 ms    | 20 ms       | 66 ms (0.3×)   | 68 ms (0.3×)   | 0.12 ms |
| ripgrep 0.1..15.2                | 252   | 68 ms    | 39 ms       | 28 ms (1.4×)   | 45 ms (0.9×)   | 0.13 ms |
| tokio 1.0..1.53                  | 867   | 119 ms   | 90 ms       | 43 ms (2.1×)   | 109 ms (0.8×)  | 0.16 ms |
| libuv v1.0..v1.53                | 495   | 96 ms    | 68 ms       | 37 ms (1.8×)   | 80 ms (0.8×)   | 0.13 ms |
| stress, deep deltas (synthetic)  | 2000  | 63 ms    | 37 ms       | 34 ms (1.1×)   | 205 ms (0.2×)  | 0.18 ms |
| 20k-line file (synthetic)        | 2002  | 68 ms    | 40 ms       | 26 ms (1.5×)   | 39 ms (1.0×)   | 0.17 ms |
| +1M lines added (synthetic)      | 2500  | 121 ms   | 94 ms       | 68 ms (1.4×)   | 120 ms (0.8×)  | 0.18 ms |
| ±250k lines modified (synthetic) | 1250  | 107 ms   | 81 ms       | 31 ms (2.6×)   | 88 ms (0.9×)   | 0.17 ms |

The `(x×)` factors are **`git internal ÷ xlr8 cold`** — >1 means xlr8 is faster.

- **Repeat views are effectively free:** ~0.1 ms from memory; compare results are also cached on disk by commit OID.
- **First views beat git on 7 of 8 ranges**, but most of the apparent lead is git's ~28 ms process spawn, which a CLI pays and a running server does not. Against git's own internal time the margin is **1.1–2.6×**, and on the smallest range (91 files) xlr8 loses outright — fixed repo-open + ref-peel cost is not amortized.
- **Multi-threaded, per-core, the range that loses outright is only rg91**; the other seven are 1.1–2.6×. Single-threaded changes that answer completely — see below.
- **Single-threaded (equal cores) xlr8 mostly loses:** 0.2–0.9× on seven ranges, within 4% on the 20k-line case. The multi-thread default is what produces the cold margin; the README's headline claim is a parallel-diffing claim, not a per-core one.
- **Known weak spots:** small ranges (fixed repo-open + ref-peel cost; cold medians there swing up to 2× between runs, so treat them as ranges) and single-threaded deep delta chains (gix's pack reader has no match for git's C delta resolver).

Full tables, per-run numbers and method: [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md).

### Also runs on a phone

xlr8 runs on Android via Termux (tested on a Snapdragon 695 / Poco X4 Pro 5G,
aarch64). Early numbers from that device are in [`docs/benchmark.md`](docs/benchmark.md);
they use an older harness and PRoot, so they are not comparable to the
table above.

## HTTP API

- `GET /api/{owner}/{repo}/refs` — branches, tags
- `GET /api/{owner}/{repo}/commits?limit=N&ref=REF&stats=1&count=1` — commit list
- `GET /api/{owner}/{repo}/diff?base=REF&head=REF` — compare summary (+/− per file)
- `GET /api/{owner}/{repo}/search?q=...` — path + grep search
- `GET /api/{owner}/{repo}/file?base=...&head=...&path=...` — file diff/hunks
- `GET /api/repos` — list mirrored repos
- `GET /api/stats` — runtime stats (RSS, CPU, cache, uptime)

## Built with

[gix](https://github.com/GitoxideLabs/gitoxide) · [axum](https://github.com/tokio-rs/axum) · [tokio](https://github.com/tokio-rs/tokio) · [imara-diff](https://github.com/Byron/imara-diff) · vanilla HTML/JS

## License

MIT © 2026 Banshal-Yadav