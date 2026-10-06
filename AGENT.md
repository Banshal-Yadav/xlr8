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

## Build & test (this environment)

Android PRoot Ubuntu rootfs — quirks are real, don't fight them:

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
- Test end-to-end:
  ```bash
  /root/xlr8-target/debug/xlr8 octocat/Hello-World --no-open --port 7777
  curl 'http://127.0.0.1:7777/api/octocat/Hello-World/diff?base=master&head=test'
  ```
- Baseline timings (tiny repo, localhost): diff ~36-68ms, file ~29-47ms
  (mostly HTTP overhead).

## Status / roadmap

- [x] v0.1 scaffold: mirror, diff engine, refs/commits, web UI, PR list
- [ ] real big-repo benchmark (linux kernel scale) + fix what's slow
- [ ] virtualized diff rendering (only visible lines in DOM)
- [ ] PR "files changed" tab using local diff of PR head/base SHAs
- [ ] syntax highlighting (syntect, must be cached to keep ms claim)
- [ ] word-level intra-line diff, rename-aware stats tuning
- [ ] diff chunk cache (persist computed hunks)

## Conventions

- Errors: `anyhow` everywhere; HTTP handlers map to `(StatusCode, String)`.
- Blocking gix work runs in `spawn_blocking` — keep it that way.
- No comments in code unless explaining a non-obvious trick.
- Perf claims must be re-measured (`curl -w "%{time_total}"`), not guessed.
