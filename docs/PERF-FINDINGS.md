# Performance findings — beating git on equal cores

Investigation date: 2026-10-08. Measured on the Windows laptop (AMD Ryzen 7
7735HS, 8C/16T, NVMe), release binary, packed mirrors, warm page cache.

## TL;DR

xlr8 currently beats git **only because it uses 16 threads** where git uses 1.
Single-threaded, xlr8 is *slower* than git on every range. Two independent
bottlenecks explain it, and neither is the diff algorithm:

1. **Delta-chain inflate** (synthetic stress case) — gix's pack reader is ~6×
   slower than git at resolving delta chains.
2. **Rename scoring** (real-repo case) — xlr8 runs a full Myers diff per
   candidate pair; git uses a cheap hash estimate.

The fix that would make xlr8 genuinely faster than git per-core is **#2**
(hash-based rename scoring). See "How to beat git" below.

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

- The README "vs git CLI" column uses git's **wall time** (includes ~25–30 ms
  process startup). Against git's *internal* time the margin shrinks:
  tokio 2.6× (not 3.3×), stress 1.3× (not 1.8×).
- The "~1 ms cached" number is a **response-cache hit** — it is not computing a
  diff fast, it is not computing at all. Legitimate for a web UI, but not a
  diff-speed claim.
- xlr8 does more work than `git diff --numstat` (own rename detector, JSON,
  HTTP), so "faster than git" compares a parallel server doing extra work
  against a serial CLI.
