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
5. Repo mirrors live in `dirs::cache_dir()/xlr8/mirrors/<owner>_<name>.git`
   (bare). Clone once, `git fetch --all --prune` after.

## Layout

```
src/main.rs      CLI (clap), startup, serve loop
src/cli.rs       arg definitions
src/mirror.rs    RepoRef parse (URL or owner/repo), clone/fetch (shells out to git)
src/diff.rs      gix: refs, commits, tree diff, per-file line diff (diff_lines)
src/github.rs    PR list client
src/server.rs    axum routes + embedded UI
assets/index.html  UI (vanilla JS, tabs: Commits / Compare / Pulls)
```

Naming history: crate name `xlr8` was chosen after checking crates.io
availability (gitbox/pdx/zif/gbx taken; p0x/sau0n/braket free but rejected).
Rename is still possible until first `cargo publish`.

## Build & test

Cross-platform: Linux, macOS, **Windows**. No unix-only deps — keep it that
way (no `libc`/`nix`, no hardcoded `/tmp` paths; use `dirs`, `PathBuf`,
`std::process::Command`). Runtime requirement: **git on PATH**
(clone/fetch shells out to `git`; Git for Windows counts).

### Windows laptop

```powershell
# one-time: install rustup.rs + Git for Windows
cargo build --release
.\target\release\xlr8.exe octocat\Hello-World   # or owner/repo
# → opens http://127.0.0.1:<port>
```

Mirrors cache to `%LOCALAPPDATA%\xlr8\mirrors` automatically.
`GITHUB_TOKEN`/`GH_TOKEN` env var optional (PR tab rate limits).

### Android PRoot (this device) — quirks are real, don't fight them

- **`/mnt/sdcard` is noexec** → always build with:
  ```bash
  export PATH="$HOME/.cargo/bin:$PATH"
  CARGO_TARGET_DIR=/root/xlr8-target cargo build
  # binary: /root/xlr8-target/debug/xlr8
  ```
- Rust was installed manually (rustup downloads break here):
  toolchain at `~/.rustup/toolchains/stable-aarch64-unknown-linux-gnu`
- IPv6 is broken on this device → IPv4 pinned in `/etc/hosts`
  (crates.io, static.rust-lang.org, github.com, ubuntu mirrors).
  **New hosts may need pinning** if network calls hang.
- Test end-to-end (release binary — perf numbers only make sense in release):
  ```bash
  CARGO_TARGET_DIR=/root/xlr8-target cargo build --release
  /root/xlr8-target/release/xlr8 octocat/Hello-World --no-open --port 7777
  curl -w '%{time_total}\n' 'http://127.0.0.1:7777/api/stress/big/diff?base=base&head=head'
  ```
- Measured on this device (6-core, PRoot ptrace overhead, release build):

  | workload | time |
  |---|---|
  | synthetic 2000-file compare | ~3-5s (git CLI baseline: 5.6s) |
  | ripgrep 254 files +73k/-22k | 96ms |
  | ripgrep 91 files +14k/-12k | 111ms |
  | file diff 485 lines | 41ms |
  | giant 20k-line file diff | 251ms |
  | refs / commits(50) | 38ms / 21-132ms |

- The same `Cargo.lock` builds on Windows — commit lockfile changes.

## Perf gotchas (learned the hard way — don't regress)

- Phase-1 stats **must stay allocation-free**: count on `&str` slices
  (`line_stats`), never build per-line `String`s just to count.
- Blob stats are fanned over worker threads (`std::thread::spawn`, own
  `gix::open` per worker, results sorted back by original index).
  Sequential blob loading was 13.3s for 2000 files; parallel+release → ~3s.
- `gix::diff_tree_to_tree` yields **directory** Modification entries too —
  filter with `entry_mode.is_tree()` on *all* variants (Add/Del/Mod/Rewrite),
  else phantom files appear (was 2200 vs git's 2000).
- Annotated tags: `rev_parse_single` returns a tag object → use
  `Object::peel_to_commit()`, not `try_into_commit()`.
- Debug builds are ~3-4× slower here; benchmark release only.

## Status / roadmap

- [x] v0.1 scaffold: mirror, diff engine, refs/commits, web UI, PR list
- [x] benchmark + fix loop: parallel stats, tree-entry filter, tag peeling,
      verified file counts match `git diff --numstat` exactly
- [ ] smoke test on Windows laptop (`cargo build --release`, open UI)
- [ ] hunk-window file diffs (currently returns whole file — 1.3MB for a
      20k-line file; GitHub-style ±3 context lines)
- [ ] virtualized diff rendering (only visible lines in DOM)
- [ ] PR "files changed" tab using local diff of PR head/base SHAs
- [ ] syntax highlighting (syntect, must be cached to keep ms claim)
- [ ] word-level intra-line diff, rename-aware stats tuning
- [ ] diff chunk cache (persist computed hunks)
- [ ] file_diff 404 (currently 500 with message — fine, could be 404)

## Conventions

- Errors: `anyhow` everywhere; HTTP handlers map to `(StatusCode, String)`.
- Blocking gix work runs in `spawn_blocking` — keep it that way.
- No comments in code unless explaining a non-obvious trick.
- Perf claims must be re-measured (`curl -w "%{time_total}"`), not guessed.
