# Benchmarks — xlr8

Every number below was measured with `curl -w '%{time_total}'` against a
`cargo build --release` binary serving on localhost. No estimates.

## Device & environment

| | |
|---|---|
| Phone | **Poco X4 Pro 5G** |
| SoC | **Snapdragon 695** (6 cores: 2× Kryo 660 @2.2 GHz + 4× @1.7 GHz) |
| OS layer | Android → Termux → **Ubuntu PRoot container** (aarch64) |
| Storage | f2fs (`/mnt/sdcard` is **noexec**, target dir on internal storage) |
| Network | flaky; IPv6 broken (IPv4 pinned in `/etc/hosts`) |

**PRoot caveat:** every syscall is trapped by ptrace, so these numbers are a
*pessimal floor*. On a normal Linux box or the Windows laptop — no PRoot, NVMe,
higher clocks — expect substantially better. The 6-core 695 with no PRoot
should land roughly 2-5× faster.

## Test repos

- **Synthetic stress** (`bench/gen_stress.sh`): 103 commits, 2002 files
  (2000 × 40-line text files), tags `base`, `head`, `head2` (head2 adds a
  20 000-line file, of which 40 lines differ from a plain copy). Installed
  as mirror slug `stress/big`.
- **Real-world**: `BurntSushi/ripgrep` (full mirror, real tags, 15 years of
  history).

## Performance timeline (synthetic 2000-file compare)

How the hot path evolved, in order — each row is a real measurement:

| # | Stage | Change | 2000-file compare |
|---|---|---|---|
| 1 | v0.1 scaffold | debug build, sequential loop, full text diff built per file just to count | **10.31 s** (cold: 19.1 s) |
| 2 | allocation fix | count-only `line_stats` on `&str` slices, `mem::take` blob reads, no lossy `.to_string()` copies | **9.34 s** — proved allocations were *not* the bottleneck |
| 3 | instrumented | revealed blob loading = **13.43 s** of the 13.46 s stats loop; pure `line_stats` = **124 ms**; tree walk = 1.46 s | (diagnosis) |
| 4 | parallel + release | worker threads (≤8, own `gix::open` per worker, results re-sorted by original index) + `--release` | **≈3.0 s** warm (2.93–3.14 s), 7.4 s first hit |
| 5 | correct diff engine | hand-rolled window-80 matcher **overcounted wildly on real code** (see below) → replaced with **imara-diff** (the engine gitoxide uses, git's histogram/myers ported) | **≈2.4 s** — faster *and* correct |

Reference points measured along the way:

- `git diff --numstat base head` (C git, same mirror): **5.56 s**
  → xlr8's gix implementation in release beats git CLI on this device.
- Debug sequential blob loop alone: 13.43 s for 2000 files (~6 ms/blob —
  PRoot syscall amplification, not CPU).

### Correctness fixes the benchmark forced

| Bug | Symptom | Fix |
|---|---|---|
| **greedy window-80 line matcher** (home-made) | plausible on synthetic lorem-ipsum, but on real code misaligned on duplicate/shifted lines: ripgrep `standard.rs` reported **+3333/−3188 vs git's +242/−97** (46 of 90 files wrong) | replaced with `imara-diff` (histogram/myers — same family as git) for both stats and line output |
| gix tree diff yields **directory** Modification/Rewrite entries | 2200 files reported vs git's 2000 (phantom `mod_N/` rows) | `entry_mode.is_tree()` filter on **all** Change variants |
| `rev_parse_single` returns a **tag object** for annotated tags | every ripgrep ref endpoint: `14.1.1 is not a commit` | `Object::peel_to_commit()` |
| nonexistent path returned empty diff | `lines: []`, HTTP 200 | explicit error (`path not found in either ref`) |

Verification after the swap (ripgrep `14.1.1..15.1.0`, per-file join against
`git diff --numstat`):

- file count **91 = 91** ✓, totals **+3607/−1073 vs git +3603/−1069**
- **87 of 90** shared files byte-exact; 3 files off by exactly 1 line
  (Myers-vs-xdiff hunk-edge case)
- stress repo: **exact match** (2000 files, +4000/−0)

## Head-to-head: xlr8 vs git CLI (C) vs libgit2 (C)

Same device, same mirrors, same ranges. xlr8's numbers **include the HTTP
round-trip + JSON serialization**; git/libgit2 only compute.

### Worst case — synthetic 2000-file compare (3 runs each)

| Implementation | Time | Result |
|---|---|---|
| **xlr8** (gix + imara-diff, 6 threads) | **2.34 / 2.45 / 2.52 s** | 2000 files, +4000/−0 ✓ |
| git CLI `diff --numstat` (C, 1 thread) | 5.55 s | 2000 files, +4000/−0 |
| libgit2 `git_diff_tree_to_tree` (C, 1 thread) | 6.97 s | 2000 files, +4000/−0 |

→ **xlr8 is ≈2.3× faster than git CLI and ≈2.8× faster than libgit2** on this
device (parallel blob stats vs single-threaded C).

### Real world — ripgrep `0.1.0..15.2.0` (254 files, +73k/−22k)

| Implementation | Time | Files | Totals |
|---|---|---|---|
| **xlr8** | **99 ms** | 254 | +73 424 / −22 315 |
| libgit2 | 104 ms | 254 | +73 390 / −22 304 |
| git CLI | 190 ms | 252 | +72 431 / −21 345 |

Notes:

- At this scale all three are ~100-200 ms; xlr8's parallelism still wins.
- git shows 252 files because it **pairs 2 rename**s (`benches/bench.rs →
  crates/globset/benches/bench.rs`, `benchsuite → benchsuite/benchsuite`);
  gix's rewrite pass missed those pairs, so we show D+A instead (the other
  range's rename *was* paired correctly: 91 = 91 files).
- libgit2 ran with its default (renames off), hence the same 254.
- Remaining line deltas are the 3 off-by-1 hunk-edge cases above
  (~0.01% of totals).

## Final numbers (release, Poco X4 Pro / Snapdragon 695 / PRoot)

### Real-world — ripgrep

| Request | Size | Time |
|---|---|---|
| compare `0.1.0..15.2.0` (15 years) | 254 files, +73 424 / −22 315 | **99 ms** |
| compare `14.1.1..15.1.0` | 91 files, +3 607 / −1 073 | **139 ms** |
| file diff `crates/core/main.rs` | 485 lines (+2/−2) | **41 ms** |
| commits?limit=50 | 50 entries | **21 ms** |

### Synthetic stress (worst case)

| Request | Size | Time |
|---|---|---|
| compare `base..head` | 2000 files, +4 000/−0 | **≈2.4 s** (cold 6.5 s) |
| file diff giant `giant.txt` (added file) | 20 000 lines → 1.3 MB payload | **251 ms** |
| file diff small file | 40 lines | **43 ms** |
| refs | 3 refs | **38 ms** |

### Small repo — octocat/Hello-World (early debug-build numbers, for scale)

| Request | Time |
|---|---|
| compare | 36–68 ms |
| file diff | 29–47 ms |

## Known gaps (from these numbers)

1. **2000-file compare ≈2.4 s** — tree walk + blob loads dominate;
   next lever is caching stats for unchanged file pairs across requests.
2. **File diffs return the whole file** (1.3 MB for a 20 k-line add) —
   GitHub-style ±3-line hunk windows would cut both payload and diff compute.
3. Cold first request pays pack-cache warm-up (6.5 s vs 2.4 s) — could warm
   on startup.
4. **Rename pairing not at git parity**: gix missed 2 of 2 rename pairs in
   the 15-year range (shows D+A instead of R → +2 files, ~+1 k lines vs
   git); 3 files differ by exactly 1 line from git's xdiff hunk edges.
   Tuning `diff::Options` rewrites settings may close this.

## Reproduce

```bash
# 1. generate + install the stress mirror
bash bench/gen_stress.sh /tmp/stress-src
git clone --mirror /tmp/stress-src ~/.cache/xlr8/mirrors/stress_big.git

# 2. run
cargo build --release
./target/release/xlr8 octocat/Hello-World --no-open --port 7777

# 3. measure
curl -w '%{time_total}\n' -o /dev/null \
  'http://127.0.0.1:7777/api/stress/big/diff?base=base&head=head'
curl -w '%{time_total}\n' -o /dev/null \
  'http://127.0.0.1:7777/api/BurntSushi/ripgrep/diff?base=0.1.0&head=15.2.0'
```
