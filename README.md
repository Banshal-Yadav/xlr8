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

vs `git diff` (Windows laptop, warm, includes HTTP+JSON):

| Range | Files | xlr8 | git | Speedup |
|---|---|---|---|---|
| tokio 1.50..1.53 | 298 | 12.5 ms | 75 ms | **6.0×** |
| libuv v1.49..v1.53 | 194 | 13 ms | 79 ms | **5.9×** |
| ripgrep 14.1..15.1 | 91 | 9.7 ms | 54 ms | **5.6×** |
| libuv v1.0..v1.53 | 495 | 46 ms | 109 ms | **2.4×** |

Every row beats git. Cached responses: **~1 ms**.

## Built with

[gix](https://github.com/GitoxideLabs/gitoxide) · [axum](https://github.com/tokio-rs/axum) · [tokio](https://github.com/tokio-rs/tokio) · [imara-diff](https://github.com/Byron/imara-diff) · vanilla HTML/JS

## License

MIT © 2026 Banshal-Yadav
