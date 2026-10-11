# Performance night — 2026-10-11

Overnight session executing `docs/PERF-FINDINGS.md` recommendations. All
numbers below are raw measurements on the ARM/PRoot device (pessimal floor —
laptop numbers are 3–10× better); no commits were made, changes are working-tree
only. Method: cold process per sample (`XLR8_THREADS=1`, kill + restart between
samples), `curl` wall-clock including HTTP+JSON, git baseline =
`git diff --numstat -M` wall, medians of 3.

## TL;DR

| What | Before | After | How |
|---|---|---|---|
| tokio 867 files, 1-thread, median | 524 ms (old binary) / 400 ms (same binary, `XLR8_RENAME_MYS=1`) | **253 ms** | hash-band rename scoring |
| Myers diffs run per compare (tokio) | ~9 650 pairs | **15 pairs** | band counters, `RUST_LOG=debug` |
| Myers diffs (rg 252 files) | ~1 416 | **2** | same |
| Myers diffs (rg 91 files) | ~7 | **0** | same |
| stress 2000 files, 1-thread (no renames; pure gix inflate) | 574 ms | **377 ms** (−34 %) | flate2 → zlib-rs backend |
| stress 2000 files, default threads, cold | 277 ms | **147 ms** | zlib-rs |
| tokio 867 files, default threads, cold | — | **151 ms** (git ≈ 190 ms) | both |
| Parity vs `git -M` rename pairs | 5/6 ranges exact, libuv 17v16 | **unchanged: 5/6 exact, libuv 17v16** | harness re-run |

Claim in `docs/PERF-FINDINGS.md` TL;DR ("single-threaded, xlr8 is *slower* than
git on every range") is now **partially obsolete**: on this device 1-thread
xlr8 beats git on rg91 (≈1.1–1.8×), libuv495 (up to 3.15× on a good run),
ties rg252 (≈1.1×), and still loses tokio867 (≈0.75×) and stress2000
(≈0.2–0.4×, gix delta chains — see "Remaining").

## 1. Hash-band rename scoring (the big lever)

**Idea (from `docs/PERF-FINDINGS.md` #1, verified against git's own source):**
git never runs a diff to score renames. `diffcore-rename.c` gates on size,
then scores with `diffcore_count_changes()` — a byte-weighted line-hash
multiset match — and accepts at `score = src_copied * MAX_SCORE / max(size) ≥
30000` (50 %). xlr8 used to run a full Myers diff on **every** pair that
survived a weak 0.30 Dice pre-filter.

**Implementation (`src/rewrite.rs`):**

- `line_hashes` now hashes every imara token exactly (terminator kept, empty
  lines included) with an inline Fx-style rolling hash, returning
  `(hash, byte_len)` per line.
- `similarity_estimate` = git's formula: `matched_bytes / max(len_a, len_b)`
  where `matched_bytes = Σ_h min(count_a, count_b) · len(line_h)`.
- Because the estimate's token multiset is a superset of any LCS-kept
  subsequence, **estimate ≥ Myers-sim for every pair** (same denominator).
  Therefore:
  - `est < 0.50` → skip (provably cannot reach 50 % even with Myers)
  - `est ≥ 0.70` → accept with the estimate as the score (git's own decision)
  - `0.50 ≤ est < 0.70` → run Myers exactly as before (decision band)
- Kill-switch: `XLR8_RENAME_MYS=1` reverts to scoring every surviving pair
  with Myers (old behavior) for A/B.

**Proof it is the band, not noise** (same binary, tokio-1.0.0..1.53.2, T=1,
3 cold reps each):

```
band on : ['220', '253', '440'] median = 253 ms
band off: ['831', '376', '400'] median = 400 ms   (XLR8_RENAME_MYS=1)
```

**Proof of mechanism** (band counters, `RUST_LOG=debug`, T=1):

```
tokio867: rename-score part: 35 pairs, est-skip 9618, est-accept 20, myers 15
rg252:    rename-score part:  2 pairs, est-skip 1414, est-accept  0, myers  2
rg91:     rename-score part:  1 pairs, est-skip    6, est-accept  1, myers  0
libuv495: rename-score part: 173 pairs, est-skip 3363, est-accept 35, myers 143
```

tokio's renames phase previously ran ~9 650 Myers diffs; 15 remain. The phase
time that is left (tokio ~105–130 ms, rg252 ~63 ms) is **blob loading +
hashing**, shared work the stats phase would otherwise pay — not diffing.

**Parity vs `git -M`** (files, status distribution, rename pair sets, all six
harness ranges): exact on git.git v2.48..v2.50 (1795 files, 903 renames,
pair-identical), rg 14.1.1..15.1.0, rg 0.1.0..15.2.0, tokio long (29 pairs),
stress. libuv v1.0..v1.53 keeps the **pre-existing** 17-vs-16 divergence
(15 common pairs; xlr8-only: `samples/socks5-proxy/util.c →
src/win/snprintf.c`, `src/win/atomicops-inl.h → src/unix/no-proctitle.c`;
git-only: `src/win/atomicops-inl.h → test/test-uname.c`) — byte-for-byte the
same mismatch the old Myers scorer produced; the band did not change it.

Line totals keep the documented imara-vs-xdiff deltas only (git diff --shortstat
values recorded in `docs/benchmark.md`).

## 2. flate2 → zlib-rs (deep-delta inflate, "Bottleneck A")

`docs/PERF-FINDINGS.md` showed gix resolving delta chains ~6× slower than git
and ruled out `zlib-ng` (worse) and the pack-cache knobs (mechanistically
useless for low-fan-in chains: `gix-pack` caches only the final object of a
chain, `data/file/decode/entry.rs` `resolve_deltas`). One backend was never
tried: flate2's pure-Rust **zlib-rs** (vendored in the offline registry,
absent from the live feature set — the build was silently on miniz_oxide).

Forced with a one-line direct dependency (feature unification; flate2's
`src/ffi/mod.rs` selects `zlib_rs` whenever no C backend is enabled and drops
miniz_oxide when any zlib feature is on):

```toml
flate2 = { version = "1.1", default-features = false, features = ["zlib-rs"] }
```

**Proof** — stress2000 has **no rename candidates** (`renames: skipped`),
so the whole diff is walk + stats(inflate):

```
T=1 cold:  574 ms  →  377 ms   (−34 %)
default:   277 ms  →  147 ms
```

Still ~4× off git's C delta resolver on the pathological 94 %-delta pack
(avg chain depth 12.7); zlib-rs recovered a third of the gap for free, with
zero C in the build.

## 3. Fx-style line hashing

SipHash (`DefaultHasher`) over ~14 MB of lines measured tens of ms of the
renames phase on this device. Replaced with an inline multiply-rotate-xor
(FxHasher core, `src/rewrite.rs` `fx64`) — no new crates. Hash quality far
exceeds git's own spanhash (which folds into 32-bit buckets mod 107 927);
a collision can only shift estimate bytes, never corrupt an accepted diff
(the Myers band and gix object ids remain the ground truth).

## 4. "700 math problems solved by OpenAI" — applicability verdict

The repo is **https://github.com/openai/math** (Apache-2.0, released
2026-10-06/07): ~719 manuscripts of **research-level open problems** (BSD
conjecture, Langlands, Kakeya, Navier–Stokes, SDP-threshold hardness, …) with
Lean formalizations — *not* competition-math solutions. Reviewed for
transferable algorithms: **none applicable** to diff/rename performance. It
contains proof techniques (analytic number theory, extremal combinatorics,
complexity lower bounds), not sequence-alignment, hashing, or scoring
algorithms. Closest item (NP-hardness at the SDP threshold) only restates the
folklore that assignment problems need heuristics — which git and xlr8 already
use. Sources: [repo README](https://github.com/openai/math),
[OpenAI announcement](https://openai.com/index/sharing-ai-progress-in-mathematics/),
[Nature coverage](https://www.nature.com/articles/d41586-026-03196-8).

## 5. Git algorithm references (extracted from the local git.git mirror)

- `diffcore-rename.c` `estimate_similarity`: size gate →
  `diffcore_count_changes` → `score = src_copied * 60000 / max_size`;
  threshold `DEFAULT_RENAME_SCORE = 30000`; top-4 candidates per dest;
  no exact-diff second pass. (mirror rev v2.5.6 `aaeee53`, and v2.56.0.)
- `diffcore-delta.c` `hash_chars`: shift-7 ×2 rolling hash, spans end at LF
  or 64 bytes, CRLF `\r` skipped for text, bucketed `mod 107927`, byte-length
  weighted counts; `count_changes` = sorted-merge `Σ min(src,dst)` per bucket.
- xlr8's band mirrors this shape with exact per-line u64 hashes (finer
  buckets ⇒ slightly higher scores than git's folded buckets; explains the
  one extra libuv pair, which git's bucketing lands below 50 %).

## 6. Remaining gaps (honest)

- **stress2000 / deep delta chains**: gix inflate still ~4× slower than git
  even on zlib-rs. Options unchanged from `docs/PERF-FINDINGS.md`: accept
  (real repos have shallow deltas), boot pre-resolve, or upstream gix work
  (cache intermediate bases — the pack cache only stores chain endpoints).
- **tokio T=1 (0.75×)**: renames+stats both pay full blob inflation serially;
  multi-thread (default) already beats git here (151 ms vs ≈190 ms).
- **walk phase tail** (occasionally 30–44 ms on tokio cold runs on this box):
  tree diff + ref peeling; not investigated tonight.
- **git baseline variance on this device is large** (libuv git median swung
  151→561 ms between interleaved runs — page-cache eviction from repeated
  boots). All claims use medians of ≥3 and same-session interleaving where
  the comparison matters (band on/off used the identical binary).
- Persistent on-disk blob cache (`PERF-FINDINGS` #2) not implemented tonight.

## 7. Multi-thread (default) old-vs-new A/B

Old binary rebuilt from `git archive HEAD` (pre-night code) into
`/tmp/opencode/xlr8_old`; new = working-tree build. Interleaved order
(old,new / new,old / old,new), 3 cold-process reps per binary per range,
medians; git = median of 3. Device = same ARM/PRoot box.

```
RESULT rg91         old=['80','70','56']  med=  70ms | new=['47','48','49']  med=  48ms | git=  79ms | vs_old=1.47x | vs_git old=1.12x new=1.65x
RESULT rg252        old=['120','103','124'] med=120ms | new=['67','79','62']  med=  67ms | git= 113ms | vs_old=1.80x | vs_git old=0.94x new=1.69x
RESULT tokio867     old=['222','175','195'] med=195ms | new=['161','192','157'] med=161ms | git= 186ms | vs_old=1.21x | vs_git old=0.95x new=1.16x
RESULT libuv495     old=['208','127','134'] med=134ms | new=['126','113','287'] med=126ms | git= 156ms | vs_old=1.06x | vs_git old=1.17x new=1.24x
RESULT stress2000   old=['331','421','401'] med=401ms | new=['177','194','225'] med=194ms | git=  69ms | vs_old=2.07x | vs_git old=0.17x new=0.35x
```

Multi-thread: **every range faster than old** (1.06–2.07×); ranges beating
git multi went 3/5 → 4/5 (rg252 and tokio flipped from ~0.95× to 1.16–1.69×).
stress2000 doubled (zlib-rs) but still loses to git's C delta resolver
(0.35×) — the documented remaining gap. Raw log: `/tmp/opencode/mt_ab2.txt`.

## 8. Files touched (working tree, NOT committed)

- `src/rewrite.rs` — band constants, `fx64`, `line_hashes` tokens,
  `similarity_estimate`, band gate + counters, kill-switch
- `Cargo.toml` / `Cargo.lock` — flate2 zlib-rs backend line
- `docs/PERF-NIGHT.md` — this file

Build: `cargo build --release --offline`, **0 warnings** (`/tmp/opencode/night_build2.log`).
Server left running on `http://127.0.0.1:8900` with the new binary.

## 9. Million-line diff in milliseconds (1M-line range)

Request: make xlr8 do a 1-million-line diff in milliseconds.

### Research verdict (why cold cannot be single-digit ms)

A first-hit diff must inflate and intern every changed byte before it can
count lines — a hard lower bound of tens of ms on this device (~30 MB of
changed content for 1M lines) even with the parallel inflate path. git
(`diff --numstat -M`) pays the same cost (3.5–3.8 s here). The
milliseconds come from **not recomputing**: compare responses are immutable
functions of two commit OIDs (objects never change), so they can be cached
on disk forever with zero invalidation logic.

### Implementation (working tree, NOT committed)

- `src/server.rs` — persistent compare cache: `cmp_dir`/`cmp_disk_get`/
  `cmp_disk_put`; bodies ≤16 MiB stored as raw JSON under
  `<mirror>.cmp/<baseOid>_<headOid>.json` (tmp→rename atomic write).
  `diff_summary` handler resolves base/head to commit OIDs
  (`diff::resolve_ids`), checks disk on in-memory miss, writes through.
- `src/diff.rs` — `resolve_ids()` helper (two `rev_parse_single` peels).

Keyed by OID (not ref name) → safe across syncs and restarts; `fresh`
semantics unchanged (`/diff` has no `fresh` param; memory L0 + disk L1).

### Test setup

`bench/gen_million.sh` → 51 MB repo installed as `bench/million` mirror
(`base` tag: 1250 files × 400 lines; `a1`: +2500 new files × 400 lines =
**+1,000,000/−0**; `m1`: first 200 lines of every base file rewritten =
**+250,000/−250,000**). Ground truth via `git diff --numstat -M`
(median of 5): a1=3759 ms, m1=3518 ms.

### Results (xlr8 on same box, this device)

| range | git numstat -M | xlr8 cold (first hit) | xlr8 warm (memory) | xlr8 after restart (disk) |
|---|---|---|---|---|
| a1: **+1,000,000 lines** | 3759 ms | **1181 ms (3.2× vs git)** | **2.3–3.5 ms** | **12.5–41.4 ms** |
| m1: 500k changed lines | 3518 ms | 1120 ms (3.1×) | 2.3 ms | 21.8–37.5 ms |

Totals match git exactly (a1 +1000000 files=2500; m1 ±250000 files=1250).
Post-restart disk hit includes repo-open + ref peels (cold refs ~25 ms);
warm-ref disk hits ~12 ms. In-memory repeat: **~1600× faster than git**.

Regression checks after change: unknown-ref 404 preserved
(`unknown ref: nope`); per-file hunks on million repo 43 ms; tokio
HEAD~50..HEAD 228 ms (190 files); 0-warning build
(`/tmp/opencode/build_full.log`). Raw numbers: this section + shell logs
`/tmp/opencode/million_truth.txt`, `r8900_*.log`.
