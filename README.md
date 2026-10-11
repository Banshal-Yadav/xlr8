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

Absolute benchmarks on the build device (Poco X4 Pro 5G, aarch64, Termux→Ubuntu
PRoot; medians). vs `git diff --numstat -M` on the
same packed mirrors. Cold = first hit after process start + cache wipe;
restart = OID-keyed disk cache after process restart; warm = in-memory repeat.

| Range | Files | git | xlr8 cold | xlr8 restart | xlr8 warm | warm vs git |
|---|---|---|---|---|---|---|
| ripgrep 14.1..15.1 | 91 | 86 ms | 61 ms | 19 ms | 2.4 ms | **36×** |
| ripgrep 0.1..15.2 | 252 | 121 ms | 57 ms | 4.4 ms | 1.7 ms | **71×** |
| tokio 1.0..1.53 | 867 | 177 ms | 156 ms | 25 ms | 3.4 ms | **52×** |
| libuv v1.0..v1.53 | 495 | 178 ms | 107 ms | 21 ms | 3.5 ms | **51×** |
| git v2.48..v2.50 | 1795 | 707 ms | 324 ms | 19 ms | 5.9 ms | **121×** |
| **+1,000,000 lines** | 2500 | 4.5 s | 1.1 s | **12 ms** | **3.8 ms** | **1194×** |

Every range beats git warm; 8/9 beat git even cold (the exception is a
synthetic deep-delta stress repo). A **1-million-line diff** computes in ~1.1 s
cold and serves in **~4 ms warm / ~12 ms after a full restart** (persistent
OID-keyed compare cache — survives restarts, no invalidation needed). Full
tables, per-rep numbers, methodology, and correctness notes: `docs/BENCHMARKS.md`.

## Built with

[gix](https://github.com/GitoxideLabs/gitoxide) · [axum](https://github.com/tokio-rs/axum) · [tokio](https://github.com/tokio-rs/tokio) · [imara-diff](https://github.com/Byron/imara-diff) · vanilla HTML/JS

## License

MIT © 2026 Banshal-Yadav
