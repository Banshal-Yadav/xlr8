# Performance findings — beating git on equal cores

Investigation date: 2026-10-08. Measured on the Windows laptop (AMD Ryzen 7
7735HS, 8C/16T, NVMe), release binary, packed mirrors, warm page cache.

> **Update 2026-10-16 (re-measured, 9 reps, settled harness):** the TL;DR below
> was written against an earlier binary. Re-run on the current build, the
> single-threaded claim is **no longer true**: against git's *wall* time,
> 1-thread xlr8 wins 5/8 ranges (1.08–2.06×) and loses rg91 and the deep-delta
> synthetic. But against git's *internal* time (spawn excluded — the fair
> per-core test), 1-thread xlr8 wins **only 1/8** (stressgiant 1.23×) and ties
> million_mod500k (0.97×). So the conclusion the TL;DR drew is right even
> though its premise was wrong: **on equal cores git is still ahead, and the
> multi-thread default is what produces xlr8's cold margin.** Bottleneck B
> (rename scoring) below is fixed; see `docs/PERF-TUNING.md`. Full tables:
> `docs/BENCHMARKS.md`.
>
> **Update 2026-10-11:** bottleneck #2 (rename scoring) is fixed — git-style
> hash-band scoring, see `docs/PERF-TUNING.md` (Myers runs per compare: tokio
> ~9650 → 15; parity vs `git -M` unchanged). Bottleneck #1 mitigated via
> flate2/zlib-rs (−34 % inflate on the synthetic stress pack).

## TL;DR

xlr8's cold advantage over git comes mostly from **using 16 threads** where git
uses 1, plus git's ~28 ms process spawn which an already-running server does not
pay. Measured on the current build, single-threaded against git's internal time
xlr8 wins only 1 of 8 ranges. Two independent bottlenecks explained the original
gap, and neither is the diff algorithm:

1. **Delta-chain inflate** (synthetic stress case) — gix's pack reader is ~6×
   slower than git at resolving delta chains. *(still true)*
2. **Rename scoring** (real-repo case) — xlr8 ran a full Myers diff per candidate
   pair; git uses a cheap hash estimate. *(**fixed** — see `PERF-TUNING.md`)*

The fix that would make xlr8 genuinely faster than git per-core was **#2**
(hash-based rename scoring). It has been implemented; #1 remains.

## Method

- `XLR8_THREADS=1` to force single-threaded, `RUST_LOG=debug` for phase timings.
- git internal time via `GIT_TRACE_PERFORMANCE=1` (excludes process startup,
  which is ~25–30 ms and flatters xlr8 in the README's wall-clock comparison).

## Single-threaded breakdown

| Range | xlr8 1-thread | git internal | dominant phase |
|---|---|---|---|
| stress 2000 files | 266 ms | 43 ms | **stats 259 ms** (blob inflate) |
| ripgrep 252 files | 76 ms | 40 ms | **renames 71 ms** |
| tokio 867 files | 150 ms | 92 ms | **renames 99 ms**, stats 46 ms |

For reference, 16-thread (default) numbers: stress 39 ms, rg 25 ms, tokio 36 ms.

## Bottleneck A — delta-chain inflate (stress)

Proved by repacking the *same* stress data with no deltas:

| Pack | stats time |
|---|---|
| delta pack (94% deltas, avg depth 12.7, max 40) | **258 ms** |
| no-delta pack (same 2000 files) | **35 ms** |

git resolves those same chains in 43 ms → **gix's pack reader is ~6× slower
than git at delta resolution.**

Knobs tested, all ineffective or worse:

| Knob | Result |
|---|---|
| `zlib-ng` backend (`gix` feature `max-performance`) | **worse** — 386 ms vs 258 ms |
| `GIX_PACK_CACHE_MEMORY=256MB` | no change (265 ms) |
| `core.deltaBaseCacheLimit=512m` | no change (273 ms) |

This is upstream gix, not xlr8 code. Real repos have shallow deltas, so this is
mostly a synthetic-fixture artifact — but it is real.

## Bottleneck B — rename scoring (real repos)

`score_part` in `src/rewrite.rs` runs a **full Myers diff on every candidate
pair** to compute the similarity score. git does *not*: it uses a cheap
hash-based estimate (`diffcore_count_changes` / spanhash) and only runs the real
diff when the estimate is near the threshold.

xlr8 already computes `line_hashes` for the pre-filter (`multiset_similarity`),
then discards it and runs Myers anyway.

## How to beat git on equal cores — ranked

1. **Replace Myers-based rename scoring with hash-based similarity** (biggest
   lever, real repos). Use the existing `line_hashes` / `multiset_similarity`
   as the score and skip Myers for the vast majority of pairs — git's own
   approach. Expected: renames 71→~10 ms (rg), 99→~15 ms (tokio). Would put
   single-threaded rg at ~15 ms vs git's 40 ms.
   - Risk: the hash estimate differs slightly from Myers, so pair selection may
     shift. Re-verify against git's pairs (the harness already exists — see
     `benchmark.md` "Git-exactness").
   - Contained change in `src/rewrite.rs`.

2. **Persistent on-disk blob cache** keyed by `(mirror, object-id)`. The
   in-memory cache (64 MB) is wiped on sync and lost on restart, so every cold
   process re-inflates. A disk cache makes warm restarts skip inflate entirely.
   Does not help the very first compute, but kills the "cold-proc" penalty.

3. **Delta inflate (stress)**: no clean fix without patching gix. Options:
   (a) accept it (real repos have shallow deltas); (b) pre-resolve blobs at boot
   for the default range; (c) contribute a faster delta resolver upstream.

## Notes on the README comparison

- The README/BENCHMARKS `git` column uses git's **wall time** (includes ~28 ms
  process startup, measured: 27.2–34.2 ms across ranges on this box). Against
  git's *internal* time (`GIT_TRACE_PERFORMANCE`) the margins shrink sharply —
  e.g. tokio867 4.3× → 2.1×, libuv495 2.6× → 1.8×, and **rg91 inverts to
  a loss (0.7× → 0.3×)**. Both columns are now published in
  `docs/BENCHMARKS.md`; use the internal column for any per-core claim.
- The "~0.1 ms cached" number is a **response-cache hit** — it is not computing a
  diff fast, it is not computing at all. Legitimate for a web UI, but not a
  diff-speed claim.
- xlr8 does more work than `git diff --numstat` (own rename detector, JSON,
  HTTP), so "faster than git" compares a parallel server doing extra work
  against a serial CLI.
- Cold measurements are less reproducible than warm ones: whole-pass medians
  move up to ~2× between passes for small/synthetic ranges (see "Cold-start
  variance" in `docs/BENCHMARKS.md`). Publish medians of ≥3 passes; never quote
  a single run.
