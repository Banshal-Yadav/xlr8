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
OID-keyed disk cache after process restart; warm = in-memory repeat.

| Range | Files | git | xlr8 cold | xlr8 restart | xlr8 warm | warm vs git |
|---|---|---|---|---|---|---|
| ripgrep 14.1..15.1 | 91 | 94 ms | 24 ms | 4.6 ms | 3.3 ms | **29×** |
| ripgrep 0.1..15.2 | 252 | 124 ms | 49 ms | 2.5 ms | 2.2 ms | **56×** |
| tokio 1.0..1.53 | 867 | 234 ms | 62 ms | 7.4 ms | 1.9 ms | **125×** |
| libuv v1.0..v1.53 | 495 | 196 ms | 47 ms | 15 ms | 2.0 ms | **101×** |
| stress 2000 files | 2000 | 122 ms | 63 ms | 4.7 ms | 2.1 ms | **59×** |
| **+1,000,000 lines** | 2500 | 205 ms | 26 ms | 19 ms | 3.1 ms | **67×** |

Every range shown beats git warm; **all beat git even cold** (1.9–7.9×). A
**1-million-line diff** computes in ~26 ms cold and serves in **~3 ms warm /
~19 ms after a full restart** (persistent OID-keyed compare cache — survives
restarts, no invalidation needed). Totals are exact on every synthetic range
and within 0.12% on real repos. Full 8-range tables, per-rep numbers,
methodology, and correctness notes: `docs/BENCHMARKS.md`.

## Built with

[gix](https://github.com/GitoxideLabs/gitoxide) · [axum](https://github.com/tokio-rs/axum) · [tokio](https://github.com/tokio-rs/tokio) · [imara-diff](https://github.com/Byron/imara-diff) · vanilla HTML/JS

## License

MIT © 2026 Banshal-Yadav
