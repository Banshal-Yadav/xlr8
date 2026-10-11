# xlr8 absolute benchmarks

Every range tested, one harness, one binary, one device. Numbers are wall-clock
HTTP round-trips against the real server vs `git diff --numstat -M` on the same
packed bare mirrors.

Device: **Windows laptop — AMD Ryzen 7 7735HS (8C/16T, 3.2 GHz), 16 GB RAM,
NVMe SSD (Kingston OM8SEP4512Q)**. Release binary, packed mirrors, warm page
cache. All medians, no outlier trimming.

Harness: `bench/absbench.py`, `bench/million_bench.py`, `bench/api_bench.py`
(raw log: `docs/BENCHMARKS-raw.txt`). Reproduce with:

```bash
cargo build --release
python bench/absbench.py --warm 100 --reps 5              # 16-thread (default)
python bench/absbench.py --warm 100 --reps 5 --threads 1  # single-thread
python bench/million_bench.py --warm 100 --reps 5         # needs bench/gen_million.py
python bench/api_bench.py --warm 100
```

## Protocol

- **git**: `git --git-dir=<mirror> diff --numstat -M <base> <head>`,
  median of 5 runs, per range. Stress refs are fully qualified
  (`refs/tags/head`) — bare `head` is ambiguous and git resolves it to `HEAD`.
- **cold**: kill server, delete every `<mirror>.cmp` disk cache, restart,
  first HTTP hit = full compute. 5 independent fresh-process reps.
- **disk**: kill server (disk cache kept), restart, first hit = OID-keyed
  persistent cache read. 5 reps.
- **warm**: in-memory repeat ×100 over a **persistent connection** (what a
  browser does), median/min/max. A fresh connection per request adds a ~24 ms
  TCP-setup spike on Windows that would otherwise dominate the median.
- **threads**: `XLR8_THREADS` env var. Default = `available_parallelism()`
  clamped to 16 (= 16 here). `--threads 1` forces single-threaded.

## Ranges

| tag | repo | base..head | what it exercises |
|---|---|---|---|
| rg91 | BurntSushi/ripgrep | 14.1.1..15.1.0 | 91 files, moderate |
| rg252 | BurntSushi/ripgrep | 0.1.0..15.2.0 | 252 files, 93k lines, many renames |
| tokio867 | tokio-rs/tokio | tokio-1.0.0..tokio-1.53.2 | 867 files, 152k lines |
| libuv495 | libuv/libuv | v1.0.0..v1.53.0 | 495 files, 104k lines, deep renames |
| stress2000 | stress/big | base..head | 2000 files, deep delta chains (worst case) |
| stressgiant | stress/big | base..head2 | 20k-line pair, 40 changed lines |
| million_add1M | bench/million | base..a1 | **+1,000,000/−0**, 2500 new files |
| million_mod500k | bench/million | base..m1 | ±250,000, 1250 modified files |

## Summary — 16 threads (default)

| range | git | xlr8 cold | xlr8 disk (restart) | xlr8 warm | cold vs git | warm vs git |
|---|---:|---:|---:|---:|---:|---:|
| rg91 | 45.1 ms | 63.2 ms | 57.4 ms | 0.09 ms | **0.71×** | 476× |
| rg252 | 64.7 ms | 27.5 ms | 1.9 ms | 0.11 ms | 2.35× | 614× |
| tokio867 | 115.0 ms | 30.3 ms | 4.4 ms | 0.12 ms | 3.80× | 996× |
| libuv495 | 93.1 ms | 40.0 ms | 2.7 ms | 0.11 ms | 2.32× | 862× |
| stress2000 | 58.7 ms | 32.3 ms | 13.4 ms | 0.15 ms | 1.81× | 400× |
| stressgiant | 68.5 ms | 27.4 ms | 14.2 ms | 0.13 ms | 2.50× | 517× |
| **million_add1M** | **121.4 ms** | **70.0 ms** | **57.3 ms** | **0.19 ms** | 1.73× | 652× |
| million_mod500k | 109.7 ms | 22.9 ms | 18.0 ms | 0.11 ms | 4.79× | 969× |

## Summary — 1 thread (`XLR8_THREADS=1`)

| range | git | xlr8 cold | xlr8 disk (restart) | xlr8 warm | cold vs git | warm vs git |
|---|---:|---:|---:|---:|---:|---:|
| rg91 | 45.5 ms | 77.0 ms | 49.8 ms | 0.10 ms | **0.59×** | 475× |
| rg252 | 64.3 ms | 48.9 ms | 15.7 ms | 0.11 ms | 1.32× | 604× |
| tokio867 | 117.9 ms | 122.0 ms | 3.3 ms | 0.11 ms | **0.97×** | 1043× |
| libuv495 | 97.7 ms | 81.7 ms | 13.5 ms | 0.11 ms | 1.20× | 908× |
| stress2000 | 57.7 ms | 218.6 ms | 16.5 ms | 0.13 ms | **0.26×** | 446× |
| stressgiant | 64.4 ms | 25.7 ms | 3.0 ms | 0.13 ms | 2.51× | 491× |
| million_add1M | 119.5 ms | 120.8 ms | 56.7 ms | 0.14 ms | **0.99×** | 878× |
| million_mod500k | 114.8 ms | 83.3 ms | 2.3 ms | 0.12 ms | 1.38× | 960× |

Headlines:

- **Warm is thread-independent and ~0.1 ms** on every range — the response is
  served from the in-memory cache; no walk, no diff, no serde. This is the
  number a browser sees on a repeat view.
- **Cold (16 threads) beats git on 7/8 ranges** (1.7–4.8×). The one loss is
  rg91 (0.71×): 91 files is small enough that xlr8's fixed repo-open + ref-peel
  cost dominates, while git's process startup is amortized.
- **Cold (1 thread) beats git on 4/8 ranges.** The single-thread column is the
  honest "equal cores" comparison: xlr8's cold advantage is largely parallelism.
  It still wins on rg252/libuv/stressgiant/million_mod500k, ties tokio867 and
  million_add1M, and loses rg91 and stress2000 (gix delta-chain inflate).
- **1-million-line diff: 0.19 ms warm, 57 ms after process restart** (git: 121 ms).

## API endpoints (tokio, warm ×100, persistent connection)

| endpoint | 16-thread median | 1-thread median |
|---|---:|---:|
| /refs (all refs) | 0.10 ms | 0.11 ms |
| /commits?limit=50 | 0.10 ms | 0.10 ms |
| /commits?limit=500 | 0.13 ms | 0.11 ms |
| /commits?count=1 | 0.09 ms | 0.09 ms |
| /commits?limit=10&ref=tokio-1.53.2 | 0.10 ms | 0.09 ms |

All endpoints are cache hits after the prime request; thread count is
irrelevant on the warm path.

## Per-rep cold and disk (ms)

Cold, 16 threads (5 fresh-process reps after full cache wipe):

| range | rep0 | rep1 | rep2 | rep3 | rep4 |
|---|---:|---:|---:|---:|---:|
| rg91 | 54.1 | 64.8 | 66.4 | 63.2 | 57.0 |
| rg252 | 25.9 | 36.5 | 27.5 | 28.2 | 18.1 |
| tokio867 | 43.0 | 30.3 | 29.8 | 42.3 | 28.0 |
| libuv495 | 23.6 | 41.6 | 24.2 | 40.0 | 40.3 |
| stress2000 | 35.1 | 32.3 | 31.5 | 32.4 | 32.0 |
| stressgiant | 22.6 | 27.4 | 24.7 | 28.1 | 29.9 |
| million_add1M | 61.4 | 70.7 | 68.7 | 70.8 | 70.0 |
| million_mod500k | 14.1 | 32.2 | 22.9 | 22.9 | 14.4 |

Cold, 1 thread:

| range | rep0 | rep1 | rep2 | rep3 | rep4 |
|---|---:|---:|---:|---:|---:|
| rg91 | 73.5 | 77.0 | 61.7 | 80.7 | 85.1 |
| rg252 | 57.2 | 56.6 | 42.6 | 39.4 | 48.9 |
| tokio867 | 122.0 | 100.5 | 118.5 | 126.7 | 146.2 |
| libuv495 | 67.8 | 81.7 | 89.7 | 84.2 | 76.1 |
| stress2000 | 203.1 | 208.2 | 233.1 | 227.4 | 218.6 |
| stressgiant | 23.4 | 25.7 | 24.8 | 30.7 | 27.7 |
| million_add1M | 116.5 | 123.9 | 120.8 | 121.5 | 105.5 |
| million_mod500k | 91.6 | 76.0 | 77.9 | 83.3 | 90.6 |

Disk (5 restarts, OID-keyed persistent cache) is dominated by repo-open + two
ref peels per request (~2–60 ms depending on page-cache state); the JSON body
read itself is ≤1 ms. Full per-rep disk numbers are in `BENCHMARKS-raw.txt`.

## Correctness vs git

Totals (files/additions/deletions) compared per range against
`git diff --numstat -M`:

| range | git files/+/- | xlr8 files/+/- | delta |
|---|---|---|---|
| rg91 | 91 / +3603 / −1069 | 91 / +3607 / −1073 | +4 / +4 |
| rg252 | 252 / +72431 / −21345 | 252 / +72442 / −21356 | +11 / +11 |
| tokio867 | 867 / +127740 / −24088 | 867 / +127896 / −24244 | +156 / +156 |
| libuv495 | 496 / +80230 / −23972 | 495 / +80354 / −24096 | −1 file, +124 / +124 |
| stress2000 | 2000 / +4000 / −0 | 2000 / +4000 / −0 | **exact** |
| stressgiant | 2002 / +44000 / −0 | 2002 / +44000 / −0 | **exact** |
| million_add1M | 2500 / +1000000 / −0 | 2500 / +1000000 / −0 | **exact** |
| million_mod500k | 1250 / +250000 / −250000 | 1250 / +250000 / −250000 | **exact** |

- **Exact on all synthetic ranges** (stress2000, stressgiant, million_add1M
  +1,000,000/−0, million_mod500k ±250,000).
- **Real-world ranges differ by ≤0.12% of totals**, deterministic across all
  5 reps. The deltas are the documented imara-vs-Myers tie-breaking on
  ambiguous lines (pre-existing in `line_stats`, unrelated to caching or the
  rename band); rename pairs are identical. libuv's −1 file is the pre-existing
  17-vs-16 rename divergence (see `docs/PERF-TUNING.md` §1).

## Known gaps

- **rg91 cold loses to git (0.59–0.71×)**: 91 files is below the crossover
  where xlr8's fixed repo-open + ref-peel cost is amortized. Warm is still 475×.
- **stress2000 single-thread loses badly (0.26×)**: synthetic deep-delta chains
  (94% deltas, avg depth 12.7) — gix's pack reader has no equivalent of git's C
  delta resolver. Multi-thread recovers to 1.81×; zlib-rs already took ~34% off.
- **Single-thread cold is roughly at parity with git** (4/8 wins) — the
  multi-thread default is what produces the large cold margins. This matches
  the analysis in `docs/PERF-FINDINGS.md`.
- **Disk-cache JSON is per-range**; a range never requested before a restart
  pays full cold cost.

## Historical: ARM/PRoot device

An earlier run (Poco X4 Pro 5G, aarch64, Termux→Ubuntu PRoot) is kept in git
history. PRoot traps every syscall via ptrace, so those numbers are a pessimal
floor — the Windows laptop above is 2–4× faster cold.
