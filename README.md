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
xlr8 owner/repo --port 7782
```

Opens `http://127.0.0.1:7782`. Repo mirrored to `~/.cache/xlr8/mirrors/`, synced on request.

## What it does

- **Commits**: paginate, per-commit +/−, activity strip, author rank
- **Compare**: rename detection, edit-script diffs, windowed rendering
- **Search**: fuzzy path matching, in-repo grep, blob viewer
- **Pulls**: browse open PRs
- **Perf bar**: live latency sparkline, cache meter, RAM/CPU

## Performance

vs `git diff --numstat -M` (Windows laptop — AMD Ryzen 7 7735HS, 8C/16T, packed
mirrors, warm, xlr8 includes HTTP+JSON).
Compute = first request after process start; cached = repeat (response cache hit).

| Range | Files | xlr8 compute | xlr8 cached | git | Speedup |
|---|---|---|---|---|---|
| tokio 1.0..1.53 | 867 | 36–39 ms | 0.8–0.9 ms | 119–123 ms | **3.3×** |
| libuv v1.0..v1.53 | 495 | 28–29 ms | 0.8 ms | 99–104 ms | **3.5×** |
| stress base..head | 2000 | 38–40 ms | 0.8–1.0 ms | 69–77 ms | **1.8×** |
| ripgrep 0.1..15.2 | 252 | 25–28 ms | 0.8–0.9 ms | 68–71 ms | **2.7×** |
| tokio 1.50..1.53 | 298 | 17–18 ms | 0.8–1.0 ms | 66–69 ms | **3.9×** |
| libuv v1.49..v1.53 | 194 | 15–18 ms | 0.8–1.0 ms | 71–74 ms | **4.4×** |
| ripgrep 14.1..15.1 | 91 | 9.5–11 ms | 0.8 ms | 47–49 ms | **4.5×** |

Every row beats git. Cached responses: **~1 ms**. File counts and totals match
git; ± deltas are the documented imara-vs-xdiff alignment ambiguity — see
`docs/benchmark.md`.

## Built with

[gix](https://github.com/GitoxideLabs/gitoxide) · [axum](https://github.com/tokio-rs/axum) · [tokio](https://github.com/tokio-rs/tokio) · [imara-diff](https://github.com/Byron/imara-diff) · vanilla HTML/JS

## License

MIT © 2026 Banshal-Yadav
