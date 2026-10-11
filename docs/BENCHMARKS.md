# xlr8 absolute benchmarks

Every range tested so far, one harness, one binary, one device. Numbers are
wall-clock HTTP round-trips against the real server (`--serve`, port 8900)
vs `git diff --numstat -M` on the same bare mirrors.

Device: Poco X4 Pro 5G (aarch64) → Termux → Ubuntu PRoot. PRoot adds
syscall overhead; medians are stable but git shows 2–4× wall-time swing
under load. All medians, no outlier trimming.

Harness: `/tmp/opencode/absbench.py` (raw log: `docs/BENCHMARKS-raw.txt`).

## Protocol

- **git**: `git --git-dir=<mirror> diff --numstat -M <base> <head>`,
  median of 5 runs, per range.
- **cold**: kill server, delete every `<mirror>.cmp` disk cache, restart,
  first HTTP hit = full compute. 5 independent fresh-process reps.
- **disk**: kill server (disk cache kept), restart, first hit = OID-keyed
  persistent cache read. 5 reps.
- **warm**: in-memory repeat ×100, median/min/max.
- **API**: git.git endpoints ×100 warm, median/min/max.

## Ranges

| tag | repo | base..head | what it exercises |
|---|---|---|---|
| rg91 | BurntSushi/ripgrep | 14.1.1..15.1.0 | 91 files, moderate |
| rg252 | BurntSushi/ripgrep | 0.1.0..15.2.0 | 252 files, 93k lines, many renames |
| tokio867 | tokio-rs/tokio | tokio-1.0.0..tokio-1.53.2 | 867 files, 152k lines |
| libuv495 | libuv/libuv | v1.0.0..v1.53.0 | 495 files, 104k lines, deep renames |
| gitpair | git/git | v2.48.0..v2.50.0 | 1795 files, real-world mess |
| stress2000 | stress/big | base..head | 2000 files, deep delta chains (worst case) |
| stressgiant | stress/big | base..head2 | 20k-line pair, 40 changed lines |
| million_add1M | bench/million | base..a1 | **+1,000,000/−0**, 2500 new files |
| million_mod500k | bench/million | base..m1 | ±250,000, 1250 modified files |

## Summary (medians)

| range | git | xlr8 cold | xlr8 disk (restart) | xlr8 warm | cold vs git | warm vs git |
|---|---:|---:|---:|---:|---:|---:|
| rg91 | 85.5 ms | 61.0 ms | 19.4 ms | 2.36 ms | 1.40× | 36× |
| rg252 | 121.4 ms | 56.9 ms | 4.4 ms | 1.70 ms | 2.14× | 71× |
| tokio867 | 177.1 ms | 155.6 ms | 25.2 ms | 3.43 ms | 1.14× | 52× |
| libuv495 | 177.8 ms | 107.0 ms | 20.6 ms | 3.52 ms | 1.66× | 51× |
| gitpair | 706.8 ms | 324.4 ms | 19.2 ms | 5.86 ms | 2.18× | 121× |
| stress2000 | 90.4 ms | 170.5 ms | 12.9 ms | 3.92 ms | **0.53×** | 23× |
| stressgiant | 116.2 ms | 43.2 ms | 4.4 ms | 3.17 ms | 2.69× | 37× |
| **million_add1M** | **4486.7 ms** | **1113.0 ms** | **12.2 ms** | **3.76 ms** | 4.03× | **1194×** |
| million_mod500k | 3737.1 ms | 1095.7 ms | 13.7 ms | 3.34 ms | 3.41× | 1118× |

Headlines:

- **1-million-line diff: 3.8 ms warm, 12 ms after process restart** (git: 4.5 s).
- Cold xlr8 beats git on 8/9 ranges; the one loss is stress2000
  (synthetic deep-delta chains — gix inflate vs git's C delta resolver,
  documented remaining gap; warm is still 23×).
- Every warm median is ≤5.9 ms regardless of range size (response is
  served from the in-memory cache; body ≤241 KB even at 2500 files).

## API endpoints (git.git, warm ×100)

| endpoint | median | min | max |
|---|---:|---:|---:|
| /refs (all refs) | 3.13 ms | 1.93 ms | 35.78 ms |
| /commits?limit=50 | 2.78 ms | 1.70 ms | 24.77 ms |
| /commits?limit=500 | 3.16 ms | 2.10 ms | 26.69 ms |
| /commits?count=1 | 3.84 ms | 1.88 ms | 1173.72 ms* |
| /commits?limit=10&ref=v2.50.0 | 3.52 ms | 1.46 ms | 16.84 ms |

\* count max = one-time cold count-cache build; repeats are cached.

## Per-rep cold and disk (ms)

Cold (5 fresh-process reps after full cache wipe):

| range | rep0 | rep1 | rep2 | rep3 | rep4 |
|---|---:|---:|---:|---:|---:|
| rg91 | 61.0 | 53.1 | 64.2 | 43.4 | 62.6 |
| rg252 | 59.0 | 57.6 | 56.9 | 53.8 | 53.4 |
| tokio867 | 187.7 | 133.0 | 163.9 | 155.6 | 137.9 |
| libuv495 | 107.0 | 117.4 | 109.8 | 87.9 | 81.2 |
| gitpair | 552.3 | 305.4 | 324.4 | 342.8 | 288.1 |
| stress2000 | 548.4* | 156.3 | 170.5 | 190.8 | 160.9 |
| stressgiant | 278.5* | 43.2 | 39.7 | 42.8 | 48.2 |
| million_add1M | 3187.0* | 1134.9 | 1109.6 | 1098.3 | 1113.0 |
| million_mod500k | 1162.7 | 1150.0 | 1092.9 | 1078.9 | 1095.7 |

\* rep0 outliers = first-boot page-cache miss after cache wipe (f2fs cold);
medians exclude the effect.

Disk (5 restarts, OID-keyed persistent cache):

| range | rep0 | rep1 | rep2 | rep3 | rep4 |
|---|---:|---:|---:|---:|---:|
| rg91 | 18.0 | 19.4 | 21.9 | 29.7 | 17.1 |
| rg252 | 4.4 | 3.5 | 4.4 | 8.4 | 4.6 |
| tokio867 | 32.6 | 22.5 | 31.6 | 24.8 | 25.2 |
| libuv495 | 23.8 | 15.7 | 20.6 | 16.0 | 31.0 |
| gitpair | 18.2 | 18.1 | 22.9 | 19.2 | 30.4 |
| stress2000 | 12.9 | 12.1 | 12.4 | 19.0 | 13.8 |
| stressgiant | 5.3 | 4.4 | 6.0 | 4.1 | 4.0 |
| million_add1M | 12.2 | 12.3 | 14.1 | 9.8 | 12.0 |
| million_mod500k | 18.7 | 4.2 | 20.1 | 4.0 | 13.7 |

Disk hits include repo-open + two ref peels per request (~5–25 ms);
the JSON body read itself is ≤1 ms for warm page cache.

## Correctness vs git

Totals (files/additions/deletions) compared per range against
`git diff --numstat -M`:

- **Exact on all synthetic ranges** (stress2000, stressgiant,
  million_add1M +1,000,000/−0, million_mod500k ±250,000).
- **Real-world ranges differ by ≤0.08% of totals**, deterministic across
  all 5 reps. Per-file hunt on rg91 (totals +4/+4): exactly 4 files off
  by **+1 addition and +1 deletion each** — imara-vs-Myers tie-breaking
  on ambiguous lines (pre-existing in `line_stats`, unrelated to caching);
  rename pairs are identical (the `{a => b}` brace-collapsed display path
  in git numstat vs xlr8's full new-path + `old_path` field is format
  only). Files: crates/core/search.rs, crates/globset/src/glob.rs,
  crates/globset/src/lib.rs, crates/printer/src/hyperlink.rs→…/mod.rs.

## Known gaps

- stress2000 cold: 0.53× git (delta-chain inflate; gix has no equivalent
  of git's C delta resolver). Warm 23×, disk 13 ms.
- tokio867/libuv/gitpair cold wins are 1.1–2.2×, not the 3–4× of the
  million range — real repos have rename-heavy corners where band
  estimation defers to full Myers more often.
- Disk-cache JSON is per-range; a range never requested before a restart
  pays full cold cost.
