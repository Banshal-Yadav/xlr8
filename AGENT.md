# AGENT.md — xlr8

## What this is

**xlr8** = "GitHub in a box". A single Rust binary that mirror-clones a GitHub
repo locally and serves a fast web viewer for code, commits, diffs and PRs.

```
xlr8 owner/repo          # clone mirror → serve → open browser
xlr8 https://github.com/owner/repo
xlr8 --port 8080 --no-open <repo>    # headless
xlr8 --serve              # re-serve previously opened repos
```

## Design rules (do not break)

1. **Diffs are always computed locally** from the bare mirror via `gix`
   (gitoxide) — never via GitHub's API. This is the whole point (ms-latency).
2. **Two-phase rendering**:
   - Phase 1: file list + +/- stats (`/diff`) — must stay fast.
   - Phase 2: per-file line hunks (`/file`) — computed on demand, never
     diff the whole repo upfront.
3. **Single binary.** Web UI is `assets/index.html` embedded with
   `include_str!` — no build step, no node, no framework.
4. **GitHub API is only used for PR metadata** (`src/github.rs`), token from
   `GITHUB_TOKEN` or `GH_TOKEN`. Public rate limits apply without a token.
5. Repo mirrors live in `mirror::cache_dir()/xlr8/mirrors/<owner>_<name>.git`
   (bare; `~/.cache` on Linux, `%LOCALAPPDATA%` on Windows — no `dirs` crate).
   Clone once, `git fetch --all --prune` after.

## Layout

```
src/main.rs      CLI (hand-rolled, no clap), startup, serve loop
src/cli.rs       arg parsing + usage text
src/mirror.rs    RepoRef parse (URL or owner/repo), clone/fetch (shells out
                 to git), cache_dir(), cached_repos()
src/repo.rs      Arc<ThreadSafeRepository> cache + blob-byte cache (memo,
                 64 MB, invalidate on pull)
src/log.rs       info!/warn!/debug! logging (debug = phase timings,
                 off unless RUST_LOG=debug)
src/diff.rs      compare (walk → renames → stats), line_stats, build_hunks,
                 file_diff, commits (full date-ordered walk), refs, has_nul
src/rewrite.rs   custom parallel rename detector (see perf gotchas)
src/github.rs    PR list client
src/server.rs    axum routes + response cache (serialized JSON per
                 mirror+request, cleared on sync) + TCP_NODELAY + embedded UI
assets/index.html  UI (vanilla JS, tabs: Commits / Compare / Pulls)
bench/gen_stress.sh  synthetic 2000-file fixture generator
benchmark.md     full perf history + numbers
```

Naming history: crate name `xlr8` was chosen after checking crates.io
availability (gitbox/pdx/zif/gbx taken; p0x/sau0n/braket free but rejected).
Rename is still possible until first `cargo publish`.

## Build & test

Cross-platform: Linux, macOS, **Windows**. No unix-only deps — keep it that
way (no `libc`/`nix`, no hardcoded `/tmp` paths; use `PathBuf`,
`std::process::Command`). Runtime requirement: **git on PATH**
(clone/fetch shells out to `git`; Git for Windows counts).

Release profile: `lto="fat"`, `codegen-units=1`, `strip=true`,
`panic="abort"` — binary ≈ **5.1 MB**. Keep it that way: dependency changes
need a reason.

### Windows laptop

```powershell
# one-time: install rustup.rs + Git for Windows
cargo build --release
.\target\release\xlr8.exe octocat\Hello-World   # or owner/repo
# → opens http://127.0.0.1:<port>
```
Mirrors cache to `%LOCALAPPDATA%\xlr8\mirrors` automatically.

### Android PRoot (this device) — quirks are real, don't fight them

- **`/mnt/sdcard` is noexec** → always build with:
  ```bash
  export PATH="$HOME/.cargo/bin:$PATH"
  CARGO_TARGET_DIR=/root/xlr8-target cargo build --release --offline
  # binary: /root/xlr8-target/release/xlr8
  ```
- Rust was installed manually (rustup downloads break here); **clippy is
  unavailable** (toolchain manifest missing + network blocked) — lint
  substitute is zero rustc warnings.
- IPv6 broken → IPv4 pinned in `/etc/hosts`. crates.io serves 403 for some
  API paths; new dependencies may be un-fetchable — `--offline` first.
- Launch/kill: start with `setsid … > log 2>&1 &`; kill by pattern only via
  split quotes (`pkill -f "xlr""8-target/release/xlr8"`) **and never in the
  same command that mentions the binary path** (pkill -f matches your own
  shell).
- Release-only perf numbers; debug is 3-4× slower.

**Full numbers and the perf history live in `benchmark.md`.** Headline
(warm, PRoot, vs `git diff --numstat -M` same device): stress 2000 files
**~96 ms vs git 143 ms**, ripgrep 15-year range **~65 ms vs 125 ms**,
rg91 **~35 ms vs 132 ms**; real repos: libuv 495 files **~90 ms vs ~230 ms**,
libuv short **~37 ms vs ~200 ms**, tokio 867 files **~139 ms vs ~217 ms**,
tokio short **~37 ms vs ~127 ms**. Repeat requests for an already-computed
range are served from the **response cache**: **1.4–12 ms** (keep-alive
hello **0.68 ms**).

## Perf gotchas (learned the hard way — don't regress)

- Line diffing (stats **and** hunk output) goes through **imara-diff**
  (`Algorithm::Myers`) — the engine gitoxide uses. Do not hand-roll diff
  algorithms: the first home-made window matcher overcounted 10-50× on real
  code (ripgrep `standard.rs`: +3333 vs git's +242). `MyersMinimal` was
  tried: identical results, no gain — stay on `Myers`.
- Stats tokenize with **`byte_lines_with_terminator`** (raw bytes): skips
  UTF-8 validation/alloc on the hot path *and* makes EOF-newline changes
  count (+1/−1) like git. Hunks use the `str` equivalent for display.
- **Blob cache** (`repo.rs`): warm requests serve blob bytes from a
  per-mirror memo (64 MB, cleared on that mirror's pull) — this alone took
  warm stress 253→96 ms. Any new blob-read path must go through
  `blob_get`/`blob_put` (`blob_bytes` in diff.rs, `load` in rewrite.rs).
- **Stats run on an atomic work queue** (`STAT_CHUNK=16`, ≤8 threads,
  results sorted back by original index). Uneven big.LITTLE cores don't
  stall the wall clock with fine-grained chunks.
- **Rename detection is ours, not gix's** (`src/rewrite.rs`): gix's rewrite
  pass missed pairs git finds. Ours: identity pass first, then similarity
  `sim = (old−removed)/max(old,new) ≥ 0.5` (f32), **global best-first
  resolution** (collect all `(dest, src, sim)` triples, sort sim desc, ties
  by index, greedy-assign with emitted check — never resolve in source-id
  order, that mispairs real repos), mode compat, `srcs*dests ≤ 32767` gate,
  sound prune `min*2 < max` (can't reach 0.5 at exact equality), one interner
  per worker with per-src token cache, binary checked once per blob at load.
  Parity: exact on all fixtures and tokio (29/29 pairs); libuv 15/16 +
  one 0.5-boundary extra pair — see benchmark.md "Git-exactness".
- `gix::diff_tree_to_tree` yields **directory** Modification entries too —
  filter with `entry_mode.is_tree()` on *all* variants (else 2200 files vs
  git's 2000).
- Annotated tags: `rev_parse_single` returns a tag object →
  `peel_to_commit()`.
- **`ThreadSafeRepository` is `!Sync`** — cache the Arc (`repo.rs`), call
  `.to_thread_local()` per thread. Handles are cheap but their gix caches
  die with the handle (that's why we own the blob cache instead).
- Commit listing must walk **all parents** in date order (BinaryHeap), not
  first-parent — merged commits were disappearing vs `git log`.
- **Loose objects on this device are ~10× slower** (PRoot FUSE): after
  generating fixtures, always `git repack -adq` the mirror.
- **Sub-ms pass gotchas** (row 14 in benchmark.md):
  - axum `Json(value)` **double-serializes** (`to_value` builds a DOM, then
    serializes it) — handlers return typed `Json<T>` or pre-serialized
    bodies from the response cache, never `Json(json!(…))`.
  - axum does **not** set `TCP_NODELAY` → `ListenerExt::tap_io` does.
  - `available_parallelism()` and `cache_dir()` are probed once (OnceLock),
    not per request.
  - Blob cache values are `Arc<[u8]>` — consumers must not clone the bytes.
  - Phase-timing log lines are `debug!` (off by default): a stderr write in
    the blocking worker lands on the response path.
  - Response cache is keyed `<mirror>\0<op>\0<params>` (32 MB cap) and
    **must be cleared in `repo::invalidate`** — add to it if you add a new
    cached endpoint.
- Perf claims must be re-measured (`curl -w "%{time_total}"`), not guessed.

## Status / roadmap

- [x] v0.1 scaffold: mirror, diff engine, refs/commits, web UI, PR list
- [x] benchmark + fix loop: parallel stats + work queue, tree-entry filter,
      tag peeling, dep diet (5.1 MB binary)
- [x] rename parity with `git diff -M` (custom detector, best-first resolve)
      — file counts, pairs exact on fixtures + tokio; libuv 15/16 + ±1
      boundary pair, ±line deltas documented in benchmark.md
- [x] real-repo validation: libuv (30 MB) + tokio (63 MB) mirror clones —
      all endpoints green, faster than git on every range (benchmark.md)
- [x] hunk-window file diffs (±3 context, giant add 251→44-66 ms)
- [x] blob cache (warm stats 175→38 ms)
- [x] EOF-newline stats/hunks, full commit walk, rename-aware `/file`
- [x] file_diff 404 (bad path/ref → 404, wrong params → 400)
- [x] sub-ms pass: typed JSON, TCP_NODELAY, Arc blob cache, response cache,
      inline micro-paths, debug-gated logs — warm endpoints 1.4–12 ms,
      outputs byte-identical to pre-pass build
- [ ] smoke test on Windows laptop (`cargo build --release`, open UI)
- [ ] virtualized diff rendering (only visible lines in DOM)
- [ ] PR "files changed" tab using local diff of PR head/base SHAs
- [ ] syntax highlighting (syntect, must be cached to keep ms claim)
- [ ] word-level intra-line diff
- [ ] startup blob-cache pre-warm (response cache covers repeats; cold
      first request still pays repo open + pack fill)

## Conventions

- Errors: `anyhow` everywhere; HTTP handlers map to `(StatusCode, String)`.
- Blocking gix work runs in `spawn_blocking` — keep it that way.
- No comments in code unless explaining a non-obvious trick.
- Perf claims must be re-measured (`curl -w "%{time_total}"`), not guessed.
- Do not commit until the user says so.
