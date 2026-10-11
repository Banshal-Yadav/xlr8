# xlr8

GitHub in a box: instant local web viewer for any repo.

## Install

```bash
git clone https://github.com/Banshal-Yadav/xlr8.git
cd xlr8
cargo build --release
```

Binary: `target/release/xlr8` (~5 MB). Requires Rust toolchain ([rustup](https://rustup.rs)).

## Usage

```bash
xlr8 owner/repo --port 7782     # open one repo
xlr8 --serve --port 7782        # serve all previously opened repos
```

Opens `http://127.0.0.1:7782`. Repo mirrored to `~/.cache/xlr8/mirrors/`, synced on request.

Flags: `--port <p>` (default 7782) · `--serve` (multi-repo, no repo arg) · `--no-open` (don't launch browser).

## What it does

- **Commits**: paginate, per-commit +/−, activity strip, author rank
- **Compare**: rename detection, edit-script diffs, windowed rendering
- **Search**: fuzzy path matching, in-repo grep, blob viewer
- **Pulls**: browse open PRs
- **Perf bar**: live latency sparkline, cache meter, RAM/CPU

## HTTP API

- `GET /api/{owner}/{repo}/refs` — branches, tags
- `GET /api/{owner}/{repo}/commits?limit=N&ref=REF&stats=1&count=1` — commit list
- `GET /api/{owner}/{repo}/diff?base=REF&head=REF` — compare summary (+/− per file)
- `GET /api/{owner}/{repo}/search?q=...` — path + grep search
- `GET /api/{owner}/{repo}/file?base=...&head=...&path=...` — file diff/hunks
- `GET /api/repos` — list mirrored repos
- `GET /api/stats` — runtime stats (RSS, CPU, cache, uptime)

## Performance

Absolute benchmarks on the build device (Windows laptop — AMD Ryzen 7 7735HS,
8C/16T, NVMe; medians, 100 warm reps). vs `git diff --numstat -M` on the same
packed mirrors. Cold = first hit after process start + cache wipe; restart =
OID-keyed disk cache after process restart; warm = in-memory repeat over a
persistent connection.

| Range | Files | git | cold (16T) | cold (1T) | restart | warm |
|---|---|---|---|---|---|---|
| ripgrep 14.1..15.1 | 91 | 45 ms | 63 ms | 77 ms | 57 ms | 0.09 ms |
| ripgrep 0.1..15.2 | 252 | 65 ms | 28 ms | 49 ms | 1.9 ms | 0.11 ms |
| tokio 1.0..1.53 | 867 | 115 ms | 30 ms | 122 ms | 4.4 ms | 0.12 ms |
| libuv v1.0..v1.53 | 495 | 93 ms | 40 ms | 82 ms | 2.7 ms | 0.11 ms |
| stress 2000 files | 2000 | 59 ms | 32 ms | 219 ms | 13 ms | 0.15 ms |
| **+1,000,000 lines** | 2500 | 121 ms | 70 ms | 121 ms | 57 ms | 0.19 ms |

**Warm is ~0.1 ms on every range** (in-memory cache hit — what a browser sees
on a repeat view). Cold with the 16-thread default beats git on 7/8 ranges
(1.7–4.8×); single-threaded (`XLR8_THREADS=1`) is the honest equal-cores
comparison and wins 4/8 — xlr8's cold margin is largely parallelism. A
**1-million-line diff** computes in ~70 ms cold and serves in **~0.2 ms warm /
~57 ms after a full restart** (persistent OID-keyed compare cache — survives
restarts, no invalidation needed). Totals are exact on every synthetic range
and within 0.12% on real repos. Full 8-range tables, per-rep numbers,
methodology, and correctness notes: `docs/BENCHMARKS.md`.

## Built with

[gix](https://github.com/GitoxideLabs/gitoxide) · [axum](https://github.com/tokio-rs/axum) · [tokio](https://github.com/tokio-rs/tokio) · [imara-diff](https://github.com/Byron/imara-diff) · vanilla HTML/JS

## License

MIT © 2026 Banshal-Yadav
