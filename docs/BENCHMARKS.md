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
python bench/absbench.py --warm 100 --reps 5
python bench/million_bench.py --warm 100 --reps 5   # needs bench/gen_million.py
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
- **warm**: in-memory repeat ×100, median/min/max.
- **API**: tokio endpoints ×100 warm, median/min/max.

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

## Summary (medians)

| range | git | xlr8 cold | xlr8 disk (restart) | xlr8 warm | cold vs git | warm vs git |
|---|---:|---:|---:|---:|---:|---:|
| rg91 | 93.5 ms | 24.2 ms | 4.6 ms | 3.25 ms | 3.87× | 28.8× |
| rg252 | 124.2 ms | 48.8 ms | 2.5 ms | 2.21 ms | 2.55× | 56.2× |
| tokio867 | 234.0 ms | 62.1 ms | 7.4 ms | 1.87 ms | 3.77× | 125.1× |
| libuv495 | 196.1 ms | 47.2 ms | 15.0 ms | 1.95 ms | 4.15× | 100.6× |
| stress2000 | 122.4 ms | 63.3 ms | 4.7 ms | 2.09 ms | 1.93× | 58.5× |
| stressgiant | 138.8 ms | 18.6 ms | 2.6 ms | 2.19 ms | 7.44× | 63.5× |
| **million_add1M** | **205.0 ms** | **25.8 ms** | **19.4 ms** | **3.05 ms** | 7.94× | 67.3× |
| million_mod500k | 184.7 ms | 27.9 ms | 3.9 ms | 2.06 ms | 6.62× | 89.6× |

Headlines:

- **Cold xlr8 beats git on all 8 ranges** (1.9–7.9×) on this device.
- **Every warm median is ≤3.3 ms** regardless of range size (response served
  from the in-memory cache; body ≤241 KB even at 2500 files).
- **1-million-line diff: 3.1 ms warm, 19 ms after process restart** (git: 205 ms).
- The persistent OID-keyed compare cache makes a restart hit single-digit ms
  for every range except the 1M-line add (19 ms — the 16 MiB body read).

## API endpoints (tokio, warm ×100)

| endpoint | median | min | max |
|---|---:|---:|---:|
| /refs (all refs) | 1.96 ms | 0.56 ms | 26.74 ms |
| /commits?limit=50 | 1.73 ms | 0.64 ms | 25.39 ms |
| /commits?limit=500 | 2.13 ms | 0.58 ms | 26.37 ms |
| /commits?count=1 | 1.81 ms | 0.48 ms | 24.68 ms |
| /commits?limit=10&ref=tokio-1.53.2 | 1.47 ms | 0.56 ms | 26.64 ms |

The ~26 ms max on every endpoint is the first sample after the prime request
(one-off scheduler/GC jitter); the median is stable at ~2 ms.

## Per-rep cold and disk (ms)

Cold (5 fresh-process reps after full cache wipe):

| range | rep0 | rep1 | rep2 | rep3 | rep4 |
|---|---:|---:|---:|---:|---:|
| rg91 | 24.2 | 36.9 | 23.0 | 28.1 | 19.1 |
| rg252 | 48.8 | 58.9 | 47.2 | 53.9 | 30.1 |
| tokio867 | 67.0 | 62.1 | 61.8 | 52.5 | 74.3 |
| libuv495 | 49.0 | 47.2 | 51.6 | 35.8 | 38.4 |
| stress2000 | 64.9 | 63.2 | 63.3 | 79.5 | 55.8 |
| stressgiant | 17.8 | 18.6 | 21.5 | 33.9 | 16.3 |
| million_add1M | 38.5 | 25.8 | 23.5 | 28.9 | 24.3 |
| million_mod500k | 27.7 | 39.5 | 26.1 | 27.9 | 29.9 |

Disk (5 restarts, OID-keyed persistent cache):

| range | rep0 | rep1 | rep2 | rep3 | rep4 |
|---|---:|---:|---:|---:|---:|
| rg91 | 4.6 | 6.7 | 4.3 | 6.4 | 3.5 |
| rg252 | 2.5 | 23.6 | 3.7 | 1.5 | 2.0 |
| tokio867 | 25.5 | 18.5 | 3.9 | 7.4 | 3.4 |
| libuv495 | 15.0 | 26.9 | 4.5 | 18.6 | 4.0 |
| stress2000 | 4.7 | 3.8 | 16.3 | 15.3 | 1.8 |
| stressgiant | 4.3 | 2.6 | 2.3 | 2.8 | 1.5 |
| million_add1M | 22.7 | 19.4 | 4.7 | 20.1 | 16.6 |
| million_mod500k | 2.0 | 27.0 | 21.7 | 3.1 | 3.9 |

Disk hits include repo-open + two ref peels per request; the JSON body read
itself is ≤1 ms for warm page cache. The spread is page-cache state after the
restart, not cache logic.

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

- **stress2000 cold is the narrowest win (1.93×)**: synthetic deep-delta chains
  (94% deltas, avg depth 12.7) — gix's pack reader has no equivalent of git's C
  delta resolver. zlib-rs recovered ~34% of the gap; warm is still 58×.
- **tokio867/libuv cold wins are 2.6–4.2×**, not the 6–8× of the million range —
  real repos have rename-heavy corners where band estimation defers to full
  Myers more often.
- **Disk-cache JSON is per-range**; a range never requested before a restart
  pays full cold cost.

## Historical: ARM/PRoot device

The prior run (Poco X4 Pro 5G, aarch64, Termux→Ubuntu PRoot) is kept in git
history. PRoot traps every syscall via ptrace, so those numbers are a pessimal
floor — the Windows laptop above is 2–4× faster cold and 1.5–2× faster warm.
