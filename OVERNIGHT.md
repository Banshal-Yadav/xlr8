# OVERNIGHT.md — xlr8 overnight session

## Study

Read all of src/ (main, server, repo, diff, search, rewrite, mirror, cli, log, github) and assets/index.html (698 lines). Reviewed gitweb/cgit/fsearch/difftastic/delta approaches from knowledge.

### Current architecture strengths
- Response cache (32MB) makes repeat requests ~1ms
- Blob cache (64MB, 16-way sharded) avoids pack re-inflate
- Windowed diff rendering: ~600 DOM nodes regardless of file size
- Parallel stats fill with fine-grained work queue
- Search: u64 char-mask prefilter kills non-candidates on one AND

### Ranked ideas (gain/effort)

1. **Rename candidate pruning before blob load** (HIGH/LOW): score_part loads ALL dest+src blobs before scoring. Size-ratio check needs the blobs loaded, but we can check `size_hint` from tree entry mode... actually no size hint. Instead: skip loading blobs for pairs that can never match by pre-computing size from the entry. Problem: tree entries don't carry size. Alternative: cache blob sizes separately (just the length, not content) to avoid re-inflate. Actually the blob cache already caches content. The real cost is first-time loads.

2. **Avoid double-imara in line_stats + diff_lines for file endpoint** (MED/LOW): file_diff calls diff_lines which runs imara; stats are only for the compare endpoint. Not overlapping. Skip.

3. **commit_count uses gix revision walk** (MED/LOW): current manual heap walk duplicates what gix does. Check if gix has `repo rev-list --count` equivalent... no public API. Keep.

4. **Boot-time blob prefetch for HEAD tree** (HIGH/MED): at prewarm, walk HEAD tree and load top-N blobs into cache. Reduces cold first-compare cost.

5. **Skip rename detection when no deletions** (HIGH/LOW): if changes has zero Deletion entries, no rename possible — skip detect entirely. Common case: small updates.

6. **Skip rename when candidates < 2 on either side** (HIGH/LOW): need at least 1 src + 1 dest for fuzzy. Exact-match pass still needed. Actually detect already handles this: if srcs or dests empty, skips fuzzy. But the exact-match pass still runs. Skip early if both are empty.

7. **Precompute dir_of once per path** (MED/LOW): already done via dirs vec.

8. **Reduce allocation in search index_of** (MED/MED): path building clones prefix per entry. Could use a single mutable buffer with push/pop. Saves ~45k clones on git.git.

9. **Parallelize commit_count** (LOW/LOW): not parallelizable, it's a graph walk.

10. **HTTP keep-alive / connection reuse** (MED/LOW): axum already does this.

### Bottleneck analysis from code

- `compare()` hot path: tree diff walk → rename detect → fill_stats
  - fill_stats is parallel, STAT_CHUNK=16
  - rename detect: loads all blobs, then O(n*m) Myers scoring
  - For git.git v2.48..v2.50: 1795 files, 903 renames → ~450 deletions × ~450 additions = 200k pairs to score (before dir-windowing kicks in at 32767 cross product... wait, 450×450=202500 > 32767, so same_dir_only=true)

- `commits()` with stats=1: walks limit commits, then diffs each vs parent. For limit=50, that's 50 tree-to-tree diffs + rename detection on each. Expensive.

- `search()`: index_of walks tree (~139ms cold on git.git), then mask-filter + score. grep: reads every candidate blob up to 1.2s budget.

- `refs()`: one full ref peel per ref. First hit ~187ms on git.git, then cached.

## Changes

### 1. Rename multiset pre-filter (rewrite.rs)
- Added `line_hashes()` + `multiset_similarity()` 
- `PRE_FILTER = PERCENTAGE * 0.6` (conservative: never skips a pair Myers could score ≥50%)
- Before Myers scoring, skip pairs with low multiset line overlap
- **Result**: diff v2.43..v2.50 cold: 0.636→0.491s (23% faster), v2.48..v2.50: 0.388→0.345s (11% faster)
- Parity: all 6 repos exact

### 2. Parallel commit-stats walk (diff.rs)
- Tree-diff + rename detection now runs in parallel per commit chunk
- Same `n_threads` as stats fill, chunked by `div_ceil`
- **Result**: commits stats=1 cold: 0.549→0.326s (41% faster)
- Parity: all 6 repos exact

### 3. Parallel grep (search.rs)
- Full-tree grep now splits candidates across threads
- Shared `AtomicBool` deadline flag for budget enforcement
- Owned `(path, blob_id)` pairs cloned per chunk (thread safety)
- **Result**: grep:rename_detection cold: 0.485→0.365s (25% faster)
- Parity: all 6 repos exact

### 4. Early exit for rename detection (rewrite.rs)
- Skip entire rename detection when no additions or no deletions
- Common case: single-file updates, small commits

### 5. Frontend: diff-find + file filter + keyboard shortcuts (assets/index.html)
- `#diff-find` in-app find on diff view: searches rendered VREG rows, Enter/Shift+Enter cycles matches, Escape clears, Ctrl+F focuses (browser default prevented)
- `#diff-filter` path substring filter with `diffFilesCache` + `renderDiffFiles()` re-render (no re-fetch)
- `/` focuses the filter; `1-4` switch tabs; `j`/`k` next/prev file card
- `diffInfoLast` captured so filter/find can restore metadata without re-calling `/diff`
- Fixed duplicate `findMatches` declaration (was a latent JS collision)
- `node --check` passes on extracted `<script>`; rebuilt binary (assets embedded via `include_str!`)

### 6. Search index_of: path buffer + incremental masks (search.rs)
- One reusable path buffer per tree level; only tree recursion stores a parent path (blob entries reuse the buffer)
- Incremental char masks: `child_mask = parent_mask | mask_of(filename) | char_bit('/')` when nested — O(name) not O(full_path) per entry
- **Bug found during A/B**: first version omitted the `/` bit from incremental mask → `/` search returned 116 vs correct 219 on ripgrep. Fixed by OR-ing `char_bit(b'/')` when `prefix_len > 0`. Verified 219=219.
- **Result**: search path cold/warm unchanged (~7ms API on hot OS page cache — tree-walk I/O dominates, not mask CPU). Modest CPU win on deep trees, no regression. Parity: path+grep exact on all 6 repos vs pre-overnight backup binary.

### 7. Parallel refs peel (diff.rs)
- Collect short-name → unpeeled id first (cheap), peel in parallel worker chunks
- Peel tags → first non-tag object of any kind (blob tags like GPG keys must survive; `peel_to_kind(Commit)` alone dropped `junio-gpg-pub` and libuv `pubkey-*` refs — found via A/B, fixed)
- Dedup by short name with HashMap last-wins semantics preserved (branch+tag same short name)
- **Result**: cold refs on git.git (1020 refs): **332ms → 70ms (4.7× faster)**
- Parity: all 6 repos exact name+id vs pre-overnight backup

### 8. On-disk commit_count cache (diff.rs)
- Count is immutable for a given (refspec, tip_id) — cache to `<mirrors>/count-cache/<mirror-slug>/<hash(spec+tip)>`
- **Rule-2 compliance note**: first wrote to `~/.cache/xlr8/count-cache/` (sibling of mirrors/) — violation of "only mirrors/ allowed". Fixed to live inside `mirrors/count-cache/`. `cached_repos()` only globs `*.git` so the dir is ignored.
- Tip moves on new commits → new cache key → auto-invalidates; no manual clear needed
- Best-effort write (ignores errors on read-only fs); hash via DefaultHasher (no new deps)
- **Result**: git.git count cold 1455ms (writes cache) → **67ms after process restart (21× faster, disk hit)**
- Verified vs `git rev-list --count`: git.git 82443, rg 2287, tokio 4756, libuv 5798 — all exact

### 9. Prewarm HEAD search index at boot (main.rs + search.rs)
- `search::prewarm_index` public wrapper around `index_of`; called from `prewarm()` after tree walk
- Builds the full path index for each mirror's HEAD commit at boot, so first `/search` skips the ~139ms tree walk
- **Result**: first search after boot: **~139ms → 1ms** (index already resident). Boot cost +~139ms per mirror (paid once at startup, invisible to users)
- Parity: path search exact on all 6 repos; diff v2.48..v2.50 still 1795/+71795/-32313 exact

## Reverted
- **Boot-time blob prefetch** (2000 blobs from HEAD tree): made cold diff 36% SLOWER (0.36→0.52s). HEAD-tree blobs don't match compare-range blobs. Memory also ballooned 4MB→57MB. Reverted to original tree-walk-only prewarm.

## Final numbers (git.git, cold first hit after boot)
| endpoint | before overnight | after overnight |
|---|---|---|
| refs | 373ms | **70ms** |
| commits stats=1 | 549ms | **326ms** |
| search grep | 485ms | **365ms** |
| search path | ~139ms (index build) | **1ms** (prewarmed at boot) |
| commit_count | 1128ms | **67ms** (disk cache, 2nd+ process) |
| diff v2.48..v2.50 | 388ms | **345ms** |
| diff v2.43..v2.50 | 636ms | **491ms** |
| file builtin.c | 21ms | 21ms |
| blob README | 4ms | 4ms |

> **FIX-session note**: the `search path → 1ms` row only holds when booting with
> `--repo` (index prewarmed). Plain `--serve` now prewarms nothing: first search
> ≈110ms (index build), then 1ms. Boot RSS without prewarm: 1.7MB (was 36.5MB).

## Final numbers (git.git, warm median of 7)
| endpoint | before | after |
|---|---|---|
| stats | 1.1ms | 1.4ms |
| repos | 1.1ms | 1.1ms |
| commits limit=1 | 2.2ms | 3.9ms |
| commits stats=1 | 1.4ms | 6.5ms |
| refs | 1.6ms | 2.6ms |
| search path | 1.5ms | 3.0ms |
| search grep | 1.6ms | 2.5ms |
| diff warm | 1.6ms | 3.4ms |

## Parity (all exact after every change)
> **Correction (FIX session)**: the +/- line totals below are xlr8's own
> `imara_diff` numbers — verified *regression-exact* (old binary vs new binary)
> and file/rename/status sets exact vs `git diff -M`, but **not** byte-identical
> to git's xdiff line choices. Raw `git diff --shortstat` differs slightly (table
> in the FIX session section). Counts/refs/grep ARE exact vs git CLI.
- rg `0.1.0..15.2.0`: 252 files, +72442/-21356
- rg 14.1.1..15.1.0: 91 files, 1 renames, +3607/-1073
- git.git v2.48.0..v2.50.0: 1795 files, 903 renames, +71795/-32313
- tokio 1.0.0..1.53.2: 867 files, 29 renames, +127896/-24244
- libuv v1.0.0..v1.53.0: 495 files, 17 renames, +80354/-24096
- stress base..head: 2000 files, 0 renames, +4000/-0
- refs name+id exact on all 6 mirrors
- search path+grep exact on all 6 mirrors
- commit_count matches `git rev-list --count` on all 4 real mirrors

## Known remaining bottlenecks
1. **Diff renames phase** still ~50% of large compare (Myers scoring). Multiset pre-filter helped; further pruning needs size hints from tree entries (gix doesn't expose them without loading the blob).
2. **Grep budget 1.2s** caps worst-case; parallel helped but content-heavy queries still bound by blob inflate I/O.
3. **Boot time +~139ms per mirror** from search-index prewarm — acceptable (once per process), but a disk-serialized index would make it ~0.
4. **commit_count first process ever** still pays full walk (~1.4s) before writing disk cache — unavoidable cold start.

## Suggested README/benchmark.md edits (for review, not applied)
- Mention search-index prewarm + commit_count disk cache in "how it stays fast"
- Update cold-start table with new refs/search/count numbers
- Note parallel refs peel and parallel grep in the concurrency section

---

# FIX session (post-audit) — the 4 assigned tasks

## F1. Count-cache hardening (src/diff.rs)

Old cache file was a bare number, trusted blindly, keyed by hash(spec+tip)
only. Replaced with a keyed, checksummed, atomically-written format:

```
XLR8CNT1 <key_hash> <count> <checksum>
```
- **key** = `hash_hex(sorted all refs name=id + HEAD id + "BIN=" CARGO_PKG_VERSION + refspec)`
  → any ref add/remove/move/force-push/tag-delete changes the key
- **checksum** = `hash_hex("XLR8CNT1|<key>|<count>|1")`
- read: magic + key + checksum all verified, else delete file + full recompute
- write: temp file + atomic rename (no half-written files)
- location: `<mirrors>/count-cache/<slug>/<hash>` (allowed path)

Corruption tests (`git rev-list --count` = 82443 throughout):

| scenario | API returned | proof |
|---|---|---|
| planted value 99999 | 82443 | file rewritten in new format |
| new format, stale checksum | 82443 | recompute |
| truncated file | 82443 | recompute |
| empty file | 82443 | recompute |
| legacy bare number (correct value) | 82443, took_ms=1112 | full WALK happened, value not trusted |
| wrong key hash (`deadbeef…`) | 82443 | recompute |

Ref-mutation scenarios (scratch *copy* of stress_big; plain file writes only,
no git write commands; scratch deleted afterwards):

| scenario | xlr8 | git CLI |
|---|---|---|
| branch moved back, pre-sync | 24 (new cache key) | 24 |
| after sync (stale loose ref gone) | 103 | 103 |
| force-push to unrelated tip | 1 (new cache key) | 1 |
| tag exists (`head2`) | 102 | 102 |
| tag deleted from packed-refs | HTTP 404 `unknown ref: head` | `fatal: unknown revision` |
| HEAD count after tag delete | 1 | 1 |

## F2. Prewarm only the CLI-given repo (src/main.rs)

`prewarm()` now runs only when `--repo` is given. `xlr8 --serve` with no repo
argument prewarms nothing (boot still does the git fetch sync of all mirrors).

A/B (old prewarm-all gate rebuilt in `.ab/` with its own `CARGO_TARGET_DIR`):

| boot | wall to serve | RSS after serve |
|---|---|---|
| before (prewarm all 6) | 25.0s / 21.7s (2 runs) | 36.5MB / 35.2MB |
| after (prewarm none) | 17.3s / 19.2s (2 runs) | **1.7MB / 1.7MB** |
| after + `--repo git/git` | 5.8s | 24.2MB (only git.git index resident) |

Startup wall-time delta is inside sync-noise (17-25s either way) — **the solid
claim is RSS: 36.5MB → 1.7MB idle, and prewarm memory now follows the CLI arg.**

## F3. Re-measured with server-side took_ms (`fresh=1` bypasses the 32MB response cache)

| endpoint (git.git unless noted) | first hit | warm, median of 7 |
|---|---|---|
| `/refs` (1020 refs) | 67ms | median 0ms (in-process REF_CACHE) |
| `/commits?count=1` | 10ms (disk cache hit after restart) | median 10ms, min 9, p95 39 (raw 9,9,9,10,11,15,39) |
| same, cache cleared (full walk) | 1401ms | — |
| `/search` path query | 110ms (first index build) | median 1ms, p95 2 |
| `/search` (ripgrep) | 9ms | median 1ms |

**Claims dropped/qualified:**
- "search path = 1ms after boot" only holds when booting **with `--repo`**
  (index prewarmed). Plain `--serve`: first search ≈110ms, then 1ms.
- overnight grep "365ms" was a single cold sample; grep varies with OS
  page-cache state (215-313ms spread observed) — no stable single figure.
- blob-prefetch "+36%": unreproduced (feature was reverted anyway).

## F4. Rebuild + full parity vs git CLI

- `cargo build --release --offline`: **0 warnings** (`/tmp/opencode/fix2_build.log`),
  1m45s. `node --check` on extracted `<script>`: **OK**.
- Final run: `/tmp/opencode/parity_final2.py` → `parity_final2_out.txt`:
  **30 hard checks, 0 failures, 7 measured deltas.**

Hard checks (all PASS): commit counts ×4 vs `git rev-list --count`; complete
refs name→peeled-id maps ×6 vs `git for-each-ref refs/heads refs/tags`
(xlr8 intentionally omits `refs/pull/*` — GitHub-style branch/tag listing:
git.git shows 1020 of 4378 total refs, octocat 3 of 3744); grep `(path,line)`
pairs + file list + `files` field vs `git grep -n/-l -i … HEAD`; diff file
counts + status distributions + rename pair sets on 5/6 ranges; stress range
fully exact including line totals.

Measured deltas vs `git diff -M` (git column = raw `git diff --shortstat`):

| range | xlr8 +/- | git +/- | files |
|---|---|---|---|
| git.git v2.48.0..v2.50.0 | +71795/-32313 | +71735/-32253 | 1795 = 1795 |
| rg 0.1.0..15.2.0 | +72442/-21356 | +72431/-21345 | 252 = 252 |
| rg 14.1.1..15.1.0 | +3607/-1073 | +3603/-1069 | 91 = 91 |
| tokio 1.0.0..1.53.2 | +127896/-24244 | +127740/-24088 | 867 = 867 |
| libuv v1.0.0..v1.53.0 | +80354/-24096 | +80230/-23972 | 495 vs **496** |
| stress base..head | +4000/-0 | +4000/-0 | exact |

- Cause: xlr8 stats/rename scoring use `imara_diff` Myers (src/diff.rs:554,579),
  git uses xdiff — both Myers, different tie-breaks on ambiguous hunks; net
  delta is always +N/-N and small. libuv additionally pairs 3 similar small
  files differently (xlr8: `socks5-proxy/util.c→win/snprintf.c`,
  `atomicops-inl.h→unix/no-proctitle.c`; git -M: `atomicops-inl.h→test/uname.c`).
  Rename pairs identical on the other 5 ranges (903 / 2 / 1 / 29 / 0).
- **Correction of earlier parity claims**: the "+71795/-32313 exact" line items
  were regression-exact (old xlr8 vs new xlr8), i.e. xlr8's own numbers — they
  were never compared to `git diff --shortstat`. File/rename/status/count/refs/
  grep parity IS exact vs git CLI (libuv pairing excepted).
- Earlier parity harness had 3 script bugs, fixed in the final run: renames
  counted `status=="rename"` (API returns `"R"`); refs compared raw tag object
  ids against xlr8's peeled ids; grep read the `hits` field (empty for `grep:`
  queries) and ran `git grep` without a rev in a bare repo (fatal → vacuous pass).

## F5. Product fix found during parity: grep `files` field (src/search.rs)

grep-only queries returned `files: 0` while `matches` listed files. Workers now
count matched files (independent of the output cap); the response uses that
count when the path-hit count is 0. Verified: `files: 6` for
`grep:rename-detection` = `git grep -l` count. Rebuilt (0 warnings) and the
30-check parity above was run against this build.
