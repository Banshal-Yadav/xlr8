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

Reference points measured along the way:

- `git diff --numstat base head` (C git, same mirror): **5.56 s**
  → xlr8's gix implementation in release beats git CLI on this device.
- Debug sequential blob loop alone: 13.43 s for 2000 files (~6 ms/blob —
  PRoot syscall amplification, not CPU).

### Correctness fixes the benchmark forced

| Bug | Symptom | Fix |
|---|---|---|
| gix tree diff yields **directory** Modification/Rewrite entries | 2200 files reported vs git's 2000 (phantom `mod_N/` rows) | `entry_mode.is_tree()` filter on **all** Change variants |
| `rev_parse_single` returns a **tag object** for annotated tags | every ripgrep ref endpoint: `14.1.1 is not a commit` | `Object::peel_to_commit()` |
| nonexistent path returned empty diff | `lines: []`, HTTP 200 | explicit error (`path not found in either ref`) |

Verified: file counts and total `+/-` now match `git diff --numstat` exactly
(2000 files, +4000/−0 on stress `base..head`).

## Final numbers (release, Poco X4 Pro / Snapdragon 695 / PRoot)

### Real-world — ripgrep

| Request | Size | Time |
|---|---|---|
| compare `0.1.0..15.2.0` (15 years) | 254 files, +73 447 / −22 338 | **96 ms** |
| compare `14.1.1..15.1.0` | 91 files, +14 521 / −11 987 | **111 ms** |
| file diff `crates/core/main.rs` | 485 lines (+2/−2) | **41 ms** |
| commits?limit=50 | 50 entries | **21 ms** |

### Synthetic stress (worst case)

| Request | Size | Time |
|---|---|---|
| compare `base..head` | 2000 files, +4 000/−0 | **≈3.0 s** (cold 7.4 s) |
| file diff giant `giant.txt` (added file) | 20 000 lines → 1.3 MB payload | **251 ms** |
| file diff small file | 40 lines | **43 ms** |
| refs | 3 refs | **38 ms** |

### Small repo — octocat/Hello-World (early debug-build numbers, for scale)

| Request | Time |
|---|---|
| compare | 36–68 ms |
| file diff | 29–47 ms |

## Known gaps (from these numbers)

1. **2000-file compare ≈3 s** — tree walk (0.5 s) + blob loads dominate;
   next lever is caching stats for unchanged file pairs across requests.
2. **File diffs return the whole file** (1.3 MB for a 20 k-line add) —
   GitHub-style ±3-line hunk windows would cut both payload and diff compute.
3. Cold first request pays pack-cache warm-up (7.4 s vs 3 s) — could warm on
   startup.

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
