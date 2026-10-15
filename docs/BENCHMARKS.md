# xlr8 absolute benchmarks

Every range tested, one harness, one binary, one device. Numbers are HTTP
round-trips against the real server vs `git diff --numstat -M` on the same
packed bare mirrors, reported against **both** git's wall-clock time and git's
own internal time (see Protocol).

Device: **Windows laptop — AMD Ryzen 7 7735HS (8C/16T, 3.2 GHz), 16 GB RAM,
NVMe SSD (Kingston OM8SEP4512Q)**. Release binary, packed mirrors, warm page
cache. All medians, no outlier trimming.

**Every xlr8 figure below is a median of 3 independent harness passes.** Cold
compare is bimodal between passes (up to 2× swing), so a single pass is not a
publishable number — see "Cold-start variance".

Harness: `bench/absbench.py`, `bench/multipass.py`, `bench/git_internal.py`,
`bench/million_bench.py`, `bench/api_bench.py`
(raw log: `docs/BENCHMARKS-raw.txt`). Reproduce with:

```bash
cargo build --release
python bench/multipass.py --passes 3 --reps 5                 # 16-thread summary tables
python bench/multipass.py --passes 3 --reps 5 --threads 1     # single-thread summary tables
python bench/git_internal.py --reps 9                         # git wall vs own internal time
python bench/absbench.py --warm 200 --reps 9                  # per-rep cold/disk detail
python bench/million_bench.py --warm 200 --reps 5             # needs bench/gen_million.py
python bench/api_bench.py --warm 100
```

`absbench.py` waits `--settle 1.0` after the readiness probe before timing
cold/disk. Without it the first range in a pass absorbs server-startup
contention and reads ~15 ms slow (this is why older runs showed rg91 cold at
63–71 ms instead of 54 ms).

## Protocol

- **git**: `git --git-dir=<mirror> diff --numstat -M <base> <head>`,
  median of 9 runs, per range. Stress refs are fully qualified
  (`refs/tags/head`) — bare `head` is ambiguous and git resolves it to `HEAD`.
- **git (internal)**: `GIT_TRACE_PERFORMANCE=1` on the same command; the final
  `performance: N s: git command` line is git's own work, excluding process
  creation + init (~28 ms on this box). Harness: `bench/git_internal.py`.
  **Both columns are reported** because the wall-clock comparison flatters
  xlr8: git pays a fixed ~28 ms that xlr8 (an already-running server) does not.
- **cold**: kill server, delete every `<mirror>.cmp` disk cache, restart,
  wait `--settle` (default 1.0 s), first HTTP hit = full compute.
  9 independent fresh-process reps.
- **disk**: kill server (disk cache kept), restart, first hit = OID-keyed
  persistent cache read. 9 reps.
- **warm**: in-memory repeat ×200 over a **persistent connection** (what a
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

Each xlr8 column is the **median of 3 independent harness passes**
(`bench/multipass.py --passes 3`), not a single pass. The last comparison
column is `git internal ÷ xlr8 cold` — **>1 means xlr8 is faster**.

| range | git wall | git internal | xlr8 cold | git-internal ÷ cold | xlr8 disk | xlr8 warm |
|---|---:|---:|---:|---:|---:|---:|
| rg91 | 47.4 ms | 20.4 ms | 65.9 ms | **0.31×** | 46.6 ms | 0.12 ms |
| rg252 | 67.9 ms | 39.0 ms | 27.9 ms | 1.40× | 20.7 ms | 0.13 ms |
| tokio867 | 118.7 ms | 90.1 ms | 43.3 ms | 2.08× | 17.3 ms | 0.16 ms |
| libuv495 | 95.8 ms | 67.7 ms | 37.2 ms | 1.82× | 15.1 ms | 0.13 ms |
| stress2000 | 63.2 ms | 37.3 ms | 33.5 ms | 1.11× | 15.3 ms | 0.18 ms |
| stressgiant | 67.5 ms | 40.3 ms | 25.7 ms | 1.57× | 14.1 ms | 0.17 ms |
| million_add1M | 121.1 ms | 93.9 ms | 68.3 ms | 1.37× | 56.7 ms | 0.18 ms |
| million_mod500k | 107.3 ms | 80.5 ms | 31.3 ms | 2.57× | 21.4 ms | 0.17 ms |

## Summary — 1 thread (`XLR8_THREADS=1`)

| range | git wall | git internal | xlr8 cold | git-internal ÷ cold | xlr8 disk | xlr8 warm |
|---|---:|---:|---:|---:|---:|---:|
| rg91 | 45.7 ms | 20.4 ms | 67.6 ms | **0.30×** | 62.6 ms | 0.12 ms |
| rg252 | 69.4 ms | 39.0 ms | 44.8 ms | **0.87×** | 18.4 ms | 0.13 ms |
| tokio867 | 122.0 ms | 90.1 ms | 108.5 ms | **0.83×** | 17.4 ms | 0.16 ms |
| libuv495 | 97.9 ms | 67.7 ms | 80.2 ms | **0.84×** | 15.4 ms | 0.13 ms |
| stress2000 | 65.1 ms | 37.3 ms | 205.4 ms | **0.18×** | 15.4 ms | 0.18 ms |
| stressgiant | 71.5 ms | 40.3 ms | 38.6 ms | 1.04× | 14.1 ms | 0.17 ms |
| million_add1M | 120.7 ms | 93.9 ms | 120.2 ms | **0.81×** | 58.7 ms | 0.16 ms |
| million_mod500k | 108.3 ms | 80.5 ms | 87.6 ms | **0.92×** | 1.7 ms | 0.15 ms |

The 1-thread xlr8 figures are the median of 3 independent passes; git
wall/internal are medians of the same sessions.

Against git's **wall** clock this is 7/8 wins at 16 threads (rg91 0.72×) and 5/8
at 1 thread. Against git's **internal** time — the fair per-core number — it is
**7/8 at 16 threads (1.1–2.6×)** and **1/8 at 1 thread**.

Headlines:

- **Warm is thread-independent and ~0.1 ms** on every range — the response is
  served from the in-memory cache; no walk, no diff, no serde. This is the
  number a browser sees on a repeat view.
- **git's ~28 ms process spawn was inflating the wall-clock comparison.** The
  multi-thread lead over git wall is 1.7–5.3×; against git's own work it is
  1.1–2.6×. The internal column is the honest one.
- **rg91 loses on both comparisons** (0.72× wall, 0.31× internal). 91 files is
  too small to amortize xlr8's fixed repo-open + ref-peel cost. Its cold value is
  also the least stable of any range (54.6–72.6 ms across passes), so read it as
  ~66 ms ±20%.
- **1-thread on equal cores loses 7/8.** The only range xlr8 does not lose
  per-core is stressgiant (1.04×, i.e. a tie); everything else is 0.18–0.92×.
  This confirms the analysis in `docs/PERF-FINDINGS.md`: **the multi-thread
  default is what produces the cold margin, not a fundamentally faster diff
  core.**
- **1-million-line diff: 0.17 ms warm, 31–68 ms after process restart**
  (git wall 107–121 ms, internal 81–94 ms).

## Cold-start variance

Cold compare is **not reproducible inside a pass**. Whole-pass medians for the
same range swing by up to 2× (rg91 observed: 54.6 / 65.9 / 72.6 ms;
stress2000: 32.4 / 33.5 / 37.1 ms; libuv495: 26.8 / 37.2 / 38.1 ms). Causes:
the boot performs a blocking network `git fetch`, so a slow fetch delays the
first request's worker; and the process/OS scheduler is not in steady state
after a fresh start.

Consequences:

- Every number in the tables above is a **median of 3 passes**
  (`bench/multipass.py --passes 3 --reps 5`); the cross-pass spread is tabulated
  below and the raw per-rep lines are in `BENCHMARKS-raw.txt`. Do not quote a
  single pass.
- **Per-rep min/max inside one pass is not an error bar.** Use the cross-pass
  range.
- `absbench.py --settle` removes startup contention but not the fetch.

## API endpoints (tokio, warm ×100, persistent connection)

| endpoint | 16-thread median | 1-thread median |
|---|---:|---:|
| /refs (all refs) | 0.10 ms | 0.11 ms |
| /commits?limit=50 | 0.09 ms | 0.10 ms |
| /commits?limit=500 | 0.11 ms | 0.11 ms |
| /commits?count=1 | 0.10 ms | 0.09 ms |
| /commits?limit=10&ref=tokio-1.53.2 | 0.09 ms | 0.09 ms |

All endpoints are cache hits after the prime request; thread count is
irrelevant on the warm path. Full-diff endpoints on the same host: `refs`
0.11 ms, `commits` 0.12 ms, `files` 0.10 ms, `tree` 0.10 ms, `blob` 0.12 ms,
`meta` 0.10 ms, `diff` (all ranges) 0.11–0.29 ms — see the `WARM` lines in
`BENCHMARKS-raw.txt`.

## Cross-pass spread (the honest error bar)

Per-rep values inside a single pass are misleading, so this table reports
**per-pass medians** across the 3 passes that produce the summary tables above.

Cold, 16 threads:

| range | pass1 | pass2 | pass3 | median | spread |
|---|---:|---:|---:|---:|---|
| rg91 | 54.6 | 65.9 | 72.6 | 65.9 | 1.33× |
| rg252 | 21.8 | 27.9 | 33.5 | 27.9 | 1.54× |
| tokio867 | 40.4 | 43.3 | 44.3 | 43.3 | 1.10× |
| libuv495 | 26.8 | 37.2 | 38.1 | 37.2 | 1.42× |
| stress2000 | 32.4 | 33.5 | 37.1 | 33.5 | 1.15× |
| stressgiant | 11.2 | 25.7 | 27.1 | 25.7 | 2.42× |
| million_add1M | 61.6 | 68.3 | 71.3 | 68.3 | 1.16× |
| million_mod500k | 29.9 | 31.3 | 36.7 | 31.3 | 1.23× |

Cold, 1 thread:

| range | pass1 | pass2 | pass3 | median | spread |
|---|---:|---:|---:|---:|---|
| rg91 | 64.3 | 67.6 | 81.5 | 67.6 | 1.27× |
| rg252 | 43.9 | 44.8 | 51.0 | 44.8 | 1.16× |
| tokio867 | 107.0 | 108.5 | 114.7 | 108.5 | 1.07× |
| libuv495 | 72.8 | 80.2 | 87.6 | 80.2 | 1.20× |
| stress2000 | 205.3 | 205.4 | 224.2 | 205.4 | 1.09× |
| stressgiant | 27.4 | 38.6 | 42.0 | 38.6 | 1.53× |
| million_add1M | 120.2 | 120.1 | 121.6 | 120.2 | 1.01× |
| million_mod500k | 82.6 | 87.6 | 93.5 | 87.6 | 1.13× |

Spread is mostly <1.5×, except the small/cheap ranges (`stressgiant`,
`rg252`, `libuv495`) where an absolute 10–15 ms of scheduling noise is a large
*relative* swing. That is why medians-of-passes are required and why rg91's
"0.7×" should be quoted as a range, not a point.

Disk (restart, OID-keyed persistent cache) is dominated by repo-open + two
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
- **Real-world ranges differ by ≤0.15% of additions and ≤0.65% of deletions.**
  Deterministic across all reps (re-checked at 9 reps; matches the earlier
  5-rep runs exactly). The deltas are imara-vs-Myers tie-breaking on ambiguous
  lines (pre-existing in `line_stats`, unrelated to caching or the rename band).
  Rename pairs: **identical on rg91 (1), rg252 (2) and tokio867 (29/29
  pair-for-pair)**; libuv is the pre-existing 17-vs-16 divergence.
- **libuv's 17-vs-16 is a near-tie, not a 0.5-threshold case** (this corrects
  the explanation previously given here). Source `src/win/atomicops-inl.h`
  (2171 B) has two viable destinations: `test/test-uname.c` (git's pick,
  `R051`) and `src/unix/no-proctitle.c` (ours). Against the source they score
  **≈0.968 (git's, 41+/28−) vs ≈0.987 (ours, 16+/27−)** — both far above the
  0.5 gate, ~0.02 apart. git's spanhash estimate happens to order them the
  other way. The impact is 1 file in 496 (0.2%): one pair counted as
  rename+add rather than rename+rename. The `util.c → src/win/snprintf.c`
  second divergence is a genuine pair git never forms at default `-M50`.

## Known gaps

- **rg91 cold loses to git on both comparisons (0.65–0.70× wall, 0.26–0.30×
  internal)**: 91 files is below the crossover where xlr8's fixed repo-open +
  ref-peel cost is amortized. Its cold median is also the least stable of any
  range (54–68 ms across passes). Warm is unaffected (~400×).
- **Against git's internal time, multi-threaded cold still wins 7/8 (1.1–2.6×)
  but 1-thread loses 7/8.** git's ~28 ms process spawn was inflating the
  wall-clock comparison by a large factor; the honest equal-cores result is
  that git's C implementation is mostly ahead and xlr8's cold margin is
  parallelism.
- **stress2000 single-thread loses badly (0.16× internal)**: synthetic
  deep-delta chains (94% deltas, avg depth 12.7) — gix's pack reader has no
  equivalent of git's C delta resolver. Multi-thread recovers to ~1.1–1.9×
  (internal); zlib-rs already took ~34% off.
- **Disk-cache JSON is per-range**; a range never requested before a restart
  pays full cold cost.
- **Boot performs a blocking network `git fetch`**, which adds variance to
  every cold measurement on a range not previously fetched.

## Historical: ARM/PRoot device

An earlier run (Poco X4 Pro 5G, aarch64, Termux→Ubuntu PRoot) is kept in git
history. PRoot traps every syscall via ptrace, so those numbers are a pessimal
floor — the Windows laptop above is 2–4× faster cold.
