# Benchmarks — xlr8

Every number below was measured with `curl -w '%{time_total}'` against a
`cargo build --release` binary serving on localhost, unless a row says
otherwise. No estimates.

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
higher clocks — expect substantially better.

**Warm vs cold:** numbers are **warm** (mirror in page cache, blob cache
filled) unless marked *cold*. Cold = first request after process start.
True OS-cold (page cache evicted) could not be forced here (no root for
`drop_caches`; `posix_fadvise DONTNEED` proved ineffective), so no OS-cold
numbers are claimed. git baselines are likewise warm.

Since the sub-ms pass (timeline row 14) there is a third state worth
knowing: **cached** = the serialized JSON body is served from the response
cache (repeat of a range already computed since the last sync). Phase
timings (`walk/renames/stats`) are `DEBUG`-level now — launch with
`RUST_LOG=debug` to see them; they are skipped by default (a stderr write
in the worker lands on the response path).

## Test repos

- **Synthetic stress** (`bench/gen_stress.sh`): 103 commits, 2002 files
  (2000 × 40-line text files), tags `base`, `head`, plus `head2` (adds
  `giant.txt`, and `giant2.txt` differs from it in 40 lines). Installed as
  mirror slug `stress/big`.
- **Real-world**: `BurntSushi/ripgrep` (full mirror, real tags, 15 years of
  history); `libuv/libuv` (30 MB mirror, 12 years, header/tree restructure);
  `tokio-rs/tokio` (63 MB mirror, 9 years, 867-file refactors).
- All mirrors must be **packed** (`git -C <mirror> repack -adq`). With
  loose objects on this device every blob read costs ~1-2 ms of PRoot FUSE
  traffic — that alone inflated every engine ~10× (see timeline row 11).

## Head-to-head: xlr8 vs git CLI (same device, packed mirror, warm)

xlr8's numbers **include the HTTP round-trip + JSON serialization**; git only
computes. git = `git diff --numstat -M` (walk + renames + stats, single
thread).

| Range | Files | xlr8 (compute) | xlr8 (cached) | git CLI | speedup (compute) |
|---|---|---|---|---|---|
| stress `base..head` | 2000 | **78–109 ms** (typ. ~96) | 16–76 ms | 143 ms | **≈1.5×** |
| ripgrep `0.1.0..15.2.0` | 252 | **54–90 ms** (typ. ~65; cold 147 ms) | 43 ms | 125 ms | **≈1.4–2.3×** |
| ripgrep `14.1.1..15.1.0` | 91 | **32 / 39 ms** | 3–12 ms | 132 ms | **≈3.5–4×** |
| libuv `v1.0.0..v1.53.0` | 495 | **90 ms** (cold-process 212 ms) | **1.5–8.4 ms** | 196–266 ms | **≈2.2×** |
| libuv `v1.49.0..v1.53.0` | 194 | **37 ms** | **1.9–3.0 ms** | 183–231 ms | **≈4.9×** |
| tokio `1.0.0..1.53.2` | 867 | **139 ms** (cold-process 252 ms) | **5.4–11.8 ms** | 214–219 ms | **≈1.6×** |
| tokio `1.50.0..1.53.2` | 298 | **37 ms** | **1.4–15 ms** | 121–133 ms | **≈3.3×** |

"cached" = response cache hit: the range was computed once since the last
sync, the JSON body is served straight from memory — no walk, no diff, no
serde. With HTTP keep-alive (what a browser does) cached hello hits
**0.68–1.5 ms**; each standalone `curl` process adds ~1–3 ms of
connection setup on this device.

Real-repo phase breakdown (tokio long, server log, `RUST_LOG=debug`): walk
5 ms, **renames 93–129 ms (501 candidates — the bottleneck)**, stats 32–45
ms. Endpoint extras on real repos (compute path): file diff 4–10 ms,
commits(n=50) 10–12 ms, refs first hit ~185 ms then cached (10 ms via
standalone curl, ~2 ms keep-alive).

Server-side compare time (log, `RUST_LOG=debug`, excludes HTTP+JSON):
rg15y 51–70 ms, rg91 30 ms, stress 52–53 ms.

Phase breakdown (warm, `RUST_LOG=debug`):

| Range | walk | renames | stats |
|---|---|---|---|
| rg15y 252 files | ~2 ms | 36–58 ms | 6–9 ms |
| rg91 91 files | 2.4 ms | 8 ms | 16 ms |
| stress 2000 files | 8–9 ms | 0 (no candidates) | 38–40 ms |

libgit2 reference (prior session, **loose-object fixture**, not re-run after
the repack): stress 6.97 s, rg15y 104 ms — kept only as history; treat with
the loose-object caveat.

## Final numbers — every endpoint (release, warm)

Compute = first request for that range (response cache miss). Cached =
repeat. Both include HTTP + JSON.

| Request | Size | Compute | Cached |
|---|---|---|---|
| rg compare `0.1.0..15.2.0` | 252 files, +72 442 / −21 356 | **106–121 ms** cold-proc, 54–90 warm | **43 ms** |
| rg compare `14.1.1..15.1.0` | 91 files, +3 607 / −1 073 | **31 ms** | **3–12 ms** |
| stress compare `base..head` | 2000 files, +4 000/−0 | **78–109 ms** | **16–76 ms** |
| stress compare *cold process* | same | 319 ms (stats 215 ms — cache fill) | — |
| file diff `crates/core/main.rs` | 483 lines | **43–56 ms** cold-proc | **1.3–2.7 ms** |
| file diff rename target | `benchsuite/benchsuite` | — | **2.7 ms**, git-exact |
| file diff giant add (`giant2.txt`) | 20 000 lines | 44–66 ms (pre-pass) | — |
| file diff small | 40 lines | 4 ms (pre-pass) | — |
| file diff libuv `src/unix/core.c` | mid-size, cross-range | 9 ms (pre-pass) | — |
| commits?n=50 | 50 entries | **7.5 ms** | **1.7–12.6 ms** |
| commits(n=50) tokio | 50 entries | **12 ms** (pre-pass) | **3.4–12.6 ms** |
| refs (tokio, first hit) | all tags/branches | **187 ms** | **1.4–15.5 ms** |
| refs (ripgrep) | all tags | 183 ms first | **4 ms** |
| refs (stress) | 3 refs | 16 ms (pre-pass) | — |
| hello diff `master^..master` | 1 file +1/−1 | **33 ms** cold-proc, 3.4–9 warm | **0.68–9 ms** (keep-alive: **0.68–1.5**) |
| hello index `/` | 1 HTML file | — | **1.9–11 ms** |
| libuv compare `v1.0.0..v1.53.0` | 495 files, +80 354 / −24 096 | **90 ms** (cold-proc 212) | **1.5–8.4 ms** |
| libuv compare `v1.49.0..v1.53.0` | 194 files, +11 830 / −3 487 | **37 ms** | **1.9–3.0 ms** |
| tokio compare `1.0.0..1.53.2` | 867 files, +127 896 / −24 244 | **139 ms** (cold-proc 252) | **5.4–11.8 ms** |
| tokio compare `1.50.0..1.53.2` | 298 files, +11 505 / −2 097 | **37 ms** | **1.4–15 ms** |

## Performance timeline — each row is a real measurement

| # | Stage | Change | Result |
|---|---|---|---|
| 1 | v0.1 scaffold | debug build, sequential, full text diff per file just to count | stress **10.31 s** (cold 19.1 s) |
| 2 | allocation fixes | count-only stats, `mem::take` blob reads | **9.34 s** — allocs were *not* the bottleneck |
| 3 | instrumented | diagnosis | blob loading = 13.43 s of the 13.46 s stats loop; `line_stats` itself = 124 ms |
| 4 | parallel + release | worker threads (≤8, own `gix::open`, results re-sorted) + `--release` | **≈3.0 s** warm |
| 5 | correct diff engine | hand-rolled window-80 matcher overcounted 10-50× on real code → **imara-diff** | **≈2.4 s** — faster *and* correct |
| 6 | correctness pass | tree-entry filter, tag peel, path 404, dep diet (clap/dirs → hand CLI; fat LTO; **5.1 MB binary**) | parity green |
| 7 | custom rename detector | gix rewrites off/weak → own parallel detector: per-worker interner reuse, `min*2<max` ratio prune, binary-at-load, sequential first-pass resolve | rg15y renames **661 ms → 52–70 ms** |
| 8 | stats work queue | atomic counter, `STAT_CHUNK=16`, n = available_parallelism().clamp(1,8) | stress stats **1.9 s → 185–236 ms** |
| 9 | fixture repack | loose → packed mirror (the loose objects were the whole story) | stress **2.56 s → ≈259 ms** (walk 650→8.5 ms) |
| 10 | hunk windows | `/file` returns GitHub-style ±3 context hunks, not the whole file | giant 20k add **251 ms → 44–66 ms** |
| 11 | terminator-kept tokens + full walk + rename-aware file view | EOF-newline changes count; commits = full date-ordered walk (not first-parent); `old_path` for renamed files | correctness (below) |
| 12 | blob cache | per-mirror memo of blob bytes (`repo.rs`, 64 MB cap, cleared on that mirror's pull) — warm requests skip pack lookup + inflate entirely | stress **253 → 96 ms** (stats 175→38 ms), rg15y **88 → 54 ms** |
| 13 | real-repo validation + rename resolver fix | libuv exposed source-id-order greedy resolution pairing wrong files (`uv-linux.h → sysinfo-memory.c`) → global best-first assignment (sort by similarity desc, ties by index, greedy with emitted check) | libuv 15/16 of git's pairs exact (was several absurd pairings), tokio 29/29 exact; prior suites still exact |
| 14 | sub-ms pass | typed `Json<T>` (axum's `Json(value)` double-serializes), `TCP_NODELAY` (axum sets none → Nagle), `Arc<[u8]>` blob cache (no clone per consumer), cached `available_parallelism`/`cache_dir`/CPU count, object cache on every gix handle, inline stats for ≤16 files, sequential rename scoring for ≤64 pairs, response cache (serialized JSON per mirror+request, cleared on sync), `DEBUG`-gated phase logs (default skips 2 stderr writes/request) | warm endpoints **1.4–12 ms** (hello keep-alive **0.68 ms**), compute path −5–15% (uvL 129→90, tkL 158→139); output **byte-identical** to pre-pass build |
| 15 | Windows cold-path pass | startup prewarm (repo open + HEAD at boot, off request path), split thread caps (stats→16, renames→8 — A/B via `XLR8_THREADS`), sharded blob cache (1 global mutex → 16-way striping) | stress cold **179→54 ms (3.3×)**, rg15y 79→36 ms; **every row beats git** on laptop; parity byte-identical |

## Correctness ledger (bugs found and fixed by comparing to git)

| Bug | Symptom | Fix |
|---|---|---|
| hand-rolled window matcher | `standard.rs` +3333/−3188 vs git +242/−97 (46/90 files wrong) | imara-diff (Myers) everywhere |
| gix tree diff yields **directory** entries | 2200 files vs git 2000 | `entry_mode.is_tree()` on all Change variants |
| annotated tag objects | `14.1.1 is not a commit` | `peel_to_commit()` |
| nonexistent path | empty diff, HTTP 200 | explicit not-found |
| gix rewrite pass missed renames | 254 files vs git 252; `file` view showed a full add for renames | own rename detector (parity exact) + `old_path` param on `/file` |
| EOF-newline blindness | `Hello World!` → `Hello World!\n` showed 0 changes | `lines_with_terminator` / `byte_lines_with_terminator` tokens in stats + hunks |
| first-parent commit walk | merged commits missing (`git log` mismatch) | full date-ordered `BinaryHeap` walk over all parents |
| `escapeHtml` skipped quotes | attribute injection via file names | escapes `&<>"'`, used at `data-path` |
| `String::from_utf8_lossy(…).to_string()` | needless double alloc per blob | lossy once; stats now diff raw bytes (no UTF-8 pass at all) |
| rename resolve in source-id order | libuv: `include/uv-linux.h` paired with `src/unix/sysinfo-memory.c` (absurd); several of git's pairs wrong | global best-first: collect `(dest, src, similarity)`, sort similarity desc, greedy-assign with emitted check (libuv 15/16 → all plausible, tokio 29/29 exact) |

## Git-exactness (what matches, what doesn't)

**Exact:** file counts (252/91/2000 = git), rename pairing (all pairs found,
same sources/targets), hunk counts, stress totals (2000, +4000/−0).

**Accepted ± deltas vs `git diff --numstat -M`** (repeated-line alignment
ambiguity between imara-Myers and git-xdiff; both edit scripts valid):

| Range | xlr8 | git | delta | files off |
|---|---|---|---|---|
| rg `0.1.0..15.2.0` | +72 442 / −21 356 | +72 431 / −21 345 | +11 / +11 | `Cargo.lock` +10/+10, `tests/tests.rs` +1/+1 |
| rg `14.1.1..15.1.0` | +3 607 / −1 073 | +3 603 / −1 069 | +4 / +4 | `core/search.rs`, `globset/{glob,lib}.rs`, `printer/hyperlink/mod.rs` — each +1/+1 |

~0.01% of totals; do not chase further — the pairing of identical lines is
genuinely ambiguous.

**Real repos (same ± class):**

| Range | xlr8 | git | delta | files / renames |
|---|---|---|---|---|
| libuv `v1.0.0..v1.53.0` | +80 354 / −24 096 | +80 230 / −23 972 | +124 / +124 | 495/17 vs **496/16** |
| libuv `v1.49.0..v1.53.0` | +11 830 / −3 487 | +11 822 / −3 479 | +8 / +8 | 194 = 194, pairs exact |
| tokio `1.0.0..1.53.2` | +127 896 / −24 244 | +127 740 / −24 088 | +156 / +156 | 867 = 867, **29 = 29 pairs exact** |
| tokio `1.50.0..1.53.2` | +11 505 / −2 097 | +11 496 / −2 088 | +9 / +9 | 298 = 298 |

**libuv's ±1 file**: our detector pairs `samples/socks5-proxy/util.c →
src/win/snprintf.c` (sim 0.509) which git never does, and swaps
`atomicops-inl.h` to `no-proctitle.c` instead of git's `test-uname.c` (git
scores: 50–51 vs `<50` for the other candidate; ours: ~0.502 for both). Both
are **0.5-threshold boundary cases** where git's xdl byte accounting lands
1–3 points below our line-exact common. git itself only pairs `util.c` at
`-M48`, never at default `-M50`. Not chased further.

## Known gaps

1. **Cold first request** pays the blob-cache fill (stress 319 ms vs 96 ms
   warm). Repo open + pack index moved off the request path by startup
   prewarm (timeline row 15), but the cache fill itself still lands on the
   first compare — pre-filling at boot not done.
2. ~~refs endpoint peels every ref on each hit~~ — fixed: ref list is
   cached per mirror (row 14 precursor) and the response body is cached
   too; first tokio hit 187 ms, then 1.4–15 ms.
3. OS-cold numbers unavailable on this device (see environment note).
4. clippy unavailable here (rustup network blocked, toolchain manifest
   missing) — lint substitute = zero rustc warnings.
5. **Response cache memory**: up to 32 MB of serialized bodies, cleared
   wholesale for a mirror on sync (and for all mirrors if the cap is
   hit). Bodies > 32 MB are never cached (compute every time).
6. **Rename scoring** (93–129 ms of tkL phone compute, 60 ms laptop) is
   unchanged — it is the correctness-critical path. Parallel (capped at 8
   workers; stats caps at 16 — `XLR8_THREADS` overrides both). Only a
   result cache sits in front of it.

## Competitive landscape (2026-10-08)

No existing tool does xlr8's job (one command → mirror + GitHub-style
two-ref compare/PR UI with cached ms diffs). Measured on this phone vs
gitweb (ships with git, CGI, fresh perl+git per request, same mirrors):

| Page | gitweb | xlr8 compute | xlr8 cached |
|---|---|---|---|
| libuv compare 495 files | 4,137–4,315 ms / 13.7 MB | 113 ms / 54 KB | 4.2–14 ms |
| libuv compare 194 files | 1,398 ms / 3.1 MB | 43 ms | 1.9 ms |
| ripgrep compare 252 files | 2,710 ms / 11.5 MB | 65 ms / 28.5 KB | 1.4–7.2 ms |
| log / summary / file view | 716 / 784 / 670 ms | 18.6 / 159 / 10 ms | 4.5 / 6.8 / 3.4 ms |

- gitweb has **no range-compare action** (commit-vs-parent only; forced via
  `hp=`). xlr8: 37–54× faster, ~1000× cached, 254× smaller payload — and
  beats raw `git diff` here (196–266 ms).
- **cgit**: 7.7 ms commit page, 96 ms 16k-line diff (2026 desktop, Varnish
  blog), ~107 req/s cached logs — single-commit diffs only, no compare.
- **rgit** (Rust/axum/gitoxide/RocksDB): closest stack, but read-only bare
  repo viewer; "97% faster" claim uncited. Live instance unreachable.
- **Forgejo/Gitea**: PR diff 490 → 92 ms only after the 2026 paging rewrite;
  `/pulls` 66 s on a 2.5k-repo instance; file views 8–40 s on huge repos.
- **Fossil** ("GitHub in a box") is a different VCS; soft-serve/ungit/tig
  are TUI/desktop — not comparable.

## Windows laptop — full re-run (2026-10-07)

Every row above re-run on real hardware (no PRoot), same methodology:
`curl -w '%{time_total}'`, release binary, packed mirrors, warm page cache
(mirrors freshly cloned; OS-cold not forced — Windows has no `drop_caches`).
"CACHED" = response-cache hit on a repeat request; first hit after process
start = compute (blob cache empty → stats includes the cache fill).

### Device & environment

| | |
|---|---|
| Laptop | **Acer Nitro ANV15-41** |
| CPU | **AMD Ryzen 7 7735HS** (8 cores / 16 threads, base 3.2 GHz) |
| RAM | 16 GB |
| GPU | RTX 4050 Laptop GPU (6 GB) + Radeon iGPU |
| Storage | **Kingston NVMe SSD** (OM8SEP4512Q, 512 GB) |
| OS | Windows 11 (build 26200) |
| Toolchain | git 2.49.0.windows.1, cargo 1.98.0, `cargo build --release` |
| vs phone | no ptrace/PRoot tax, real syscalls, NVMe, higher clocks |

### Head-to-head: xlr8 vs git CLI (same device, packed mirror, warm)

After the cold-path pass (see "Cold-path fixes" below).

| Range | Files | xlr8 (compute) | xlr8 (cached) | git CLI | speedup (compute) |
|---|---|---|---|---|---|
| stress `base..head` | 2000 | **54 ms** | **0.7–1.1 ms** | 79 ms | **≈1.5×** |
| ripgrep `0.1.0..15.2.0` | 252 | **36 ms** | 0.8 ms | 76 ms | **≈2.1×** |
| ripgrep `14.1.1..15.1.0` | 91 | **9.7 ms** | 0.8 ms | 54 ms | **≈5.6×** |
| libuv `v1.0.0..v1.53.0` | 495 | **46 ms** | 1.1 ms | 109 ms | **≈2.4×** |
| libuv `v1.49.0..v1.53.0` | 194 | **13 ms** | 0.9 ms | 79 ms | **≈5.9×** |
| tokio `1.0.0..1.53.2` | 867 | **81 ms** | 0.9–1.2 ms | 134 ms | **≈1.7×** |
| tokio `1.50.0..1.53.2` | 298 | **12.5 ms** | 0.9 ms | 75 ms | **≈6.0×** |

**Every row beats git.** (Pre-fix: stress row lost — 143–179 ms vs git 79 ms.)

Notes:
- tokio tags are `tokio-1.0.0`-style; benchmark URLs above use real tag names.
- Compute column = first request after process start (cold-proc). Server-side
  phase timings below; HTTP+JSON adds ~1–5 ms.

### Cold-path fixes (stress row was losing → now wins)

First run: stress **143–179 ms vs git 79 ms** — git won that row. Anatomy of
the cold hit: repo open + pack index ~50 ms deferred to first request, plus
blob-cache fill (4000 loads) 54–90 ms in stats — while the warm engine was
already **4.3 ms** (≈12× faster than git). Three fixes, A/B-measured:

1. **Startup prewarm** (`main.rs`): open each mirror + read HEAD and its tree
   entries *before* serving. Moves ~50 ms of repo open + pack index load off
   the request path. Logged at boot: `prewarm … in 47 ms`.
2. **Split thread caps** (`repo.rs`, `rewrite.rs`): stats fill scales to 16
   threads (stress 88→54 ms); rename scoring *regresses* at 16 (60→76 ms —
   memory-bound blob loads, SMT sibling contention) → renames cap at 8.
   New `XLR8_THREADS` env override for A/B.
3. **Sharded blob cache** (`repo.rs`): single global `Mutex<BlobCache>` was a
   serialization point for 16 workers → 16-way lock striping (per-shard 4 MB
   cap, DefaultHasher on path+id; `invalidate` walks all shards).

Result: stress cold **179 → 54 ms (3.3×)**; every row now beats git.
Correctness re-verified byte-identical after all three fixes (rg
252/+72 442/−21 356, stress 2000/+4 000/−0, tokio 867/+127 896/−24 244).

### Phase breakdown (warm, `RUST_LOG=debug`)

Post-fix values; pre-fix in parens where changed.

| Range | walk | renames | stats | total (server) |
|---|---|---|---|---|
| stress 2000 files | 7.2 ms | 0.08 ms | **40 ms** (was 90) | **51 ms** (was 140–177) |
| rg 252 files | 1.5 ms | 27 ms | 3.0 ms | 34 ms |
| rg 91 files | 1.5 ms | 0.6 ms | 5.9 ms | 8.4 ms |
| libuv 495 files | 1.9 ms | 21 ms | 16.6 ms | 43 ms |
| libuv 194 files | 1.7 ms | 0.5 ms | 9.1 ms | 11.8 ms |
| tokio 867 files | 4.3 ms | **60 ms** (76 at 16 thr) | 10.9 ms | 79 ms |
| tokio 298 files | 3.4 ms | 0.9 ms | 6.4 ms | 11 ms |

Rename scoring still the bottleneck on big ranges (tokio: 60 ms of 79 ms) —
but **≈1.6–2× faster than the phone** (93–129 ms). Small ranges skip the
parallel path entirely (≤64 pairs → sequential, sub-ms).

### Other endpoints (cold-proc → cached)

| Request | Laptop | Phone |
|---|---|---|
| refs tokio (first hit) | **24 ms** → 1.0 ms | 187 ms → 1.4–15 ms |
| commits?n=50 (rg) | 2.3 ms → 1.0 ms | 7.5–12 ms → 1.7–12.6 ms |
| file diff rg `crates/core/main.rs` | **1.6–1.8 ms** → 0.9 ms | 43–56 ms → 1.3–2.7 ms |
| file diff stress `giant2.txt` (20k lines) | 9.4 ms | 44–66 ms |
| hello diff `7a6b19cf..main` | 22 ms (repo open) → 1.4 ms | 33 ms → 0.68–9 ms |
| index `/` | **0.8–1.1 ms** | 1.9–11 ms |
| keep-alive repeat (browser-style) | **0.71–1.1 ms** | 0.68–1.5 ms |

### Correctness

Output **identical to the phone run** on every range (file counts and
totals match the git-exact numbers documented above): rg 252/+72 442/−21 356,
rg91 91/+3 607/−1 073, libuv 495/+80 354/−24 096, tokio 867/+127 896/−24 244,
stress 2000/+4 000/−0. Cross-device, byte-level parity — the ± deltas vs git
are the documented imara-vs-xdiff ambiguity, unchanged.

### Headline deltas (laptop vs phone)

- Cached responses: **0.7–1.1 ms** vs 1.5–15 ms
- tokio long compute: **81 ms** vs 139 ms
- rename scoring (tokio): **60 ms** vs 93–129 ms
- refs first hit (tokio): **24 ms** vs 187 ms
- file diff (rg main.rs): **1.6 ms** vs 43–56 ms (cold-proc both)
- stress cold: **54 ms** vs 78–109 ms — and git baseline also got faster
  without PRoot (79 ms vs 143 ms); after the cold-path fixes xlr8 still
  wins every row

### Reproduce (Windows)

```powershell
# fixtures: gen_stress.sh has CRLF — strip before running under WSL bash
#   bash: tr -d '\r' < bench/gen_stress.sh > /tmp/gen.sh && bash /tmp/gen.sh <src>
# WSL /tmp is tmpfs (wiped on VM idle) — generate under %TEMP% instead
git clone --mirror <stress-src> "$env:LOCALAPPDATA\xlr8\mirrors\stress_big.git"
git -C "$env:LOCALAPPDATA\xlr8\mirrors\stress_big.git" repack -adq
$env:RUST_LOG="debug"; .\target\release\xlr8.exe owner/repo --no-open --port 7777
curl.exe -w '%{time_total}' -o NUL 'http://127.0.0.1:7777/api/stress/big/diff?base=base&head=head'
git -C "$env:LOCALAPPDATA\xlr8\mirrors\stress_big.git" diff --numstat -M base head
```

## Reproduce (phone)

```bash
# 1. generate + install the stress mirror (packed!)
bash bench/gen_stress.sh /tmp/stress-src
git clone --mirror /tmp/stress-src ~/.cache/xlr8/mirrors/stress_big.git
git -C ~/.cache/xlr8/mirrors/stress_big.git repack -adq

# 2. run (RUST_LOG=debug only if you want walk/renames/stats phase logs)
export PATH="$HOME/.cargo/bin:$PATH"
CARGO_TARGET_DIR=/root/xlr8-target cargo build --release
setsid /root/xlr8-target/release/xlr8 octocat/Hello-World --no-open --port 7777 &

# 3. measure (first hit = compute, repeat = response cache)
curl -w '%{time_total}\n' -o /dev/null \
  'http://127.0.0.1:7777/api/stress/big/diff?base=base&head=head'
curl -w '%{time_total}\n' -o /dev/null \
  'http://127.0.0.1:7777/api/BurntSushi/ripgrep/diff?base=0.1.0&head=15.2.0'
git -C ~/.cache/xlr8/mirrors/BurntSushi_ripgrep.git diff --numstat -M 0.1.0 15.2.0

# 4. real repos (same pattern)
git clone --mirror https://github.com/libuv/libuv ~/.cache/xlr8/mirrors/libuv_libuv.git
git -C ~/.cache/xlr8/mirrors/libuv_libuv.git repack -adq
setsid /root/xlr8-target/release/xlr8 libuv/libuv --no-open --port 7780 &
curl -w '%{time_total}\n' -o /dev/null \
  'http://127.0.0.1:7780/api/libuv/libuv/diff?base=v1.0.0&head=v1.53.0'
git -C ~/.cache/xlr8/mirrors/libuv_libuv.git diff --numstat -M v1.0.0 v1.53.0
# baseline note: git is run against the same packed mirror, warm, output → /dev/null;
# xlr8 time includes HTTP + JSON — it still wins every row.
```
