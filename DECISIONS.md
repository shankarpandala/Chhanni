# Decisions Log

Append-only. One entry per non-trivial choice. Format:

```
## YYYY-MM-DD — <short title>
**Context**: ...
**Options considered**: ...
**Decision**: ...
**Reasoning**: ...
**Reversibility**: easy | moderate | hard
```

---

## 2026-05-25 — Bootstrap

**Context**: Initial repo, no code yet.
**Decision**: Created `SPEC.md`, `PLAN.md`, `CLAUDE.md`, and these append-only ledger files (`DECISIONS.md`, `PROGRESS.md`, `BACKLOG.md`) per the bootstrap protocol.

---

## 2026-05-25 — Keychain plugin: `keyring` crate

**Context**: SPEC §2 listed both `tauri-plugin-stronghold` and the `keyring` crate. Need a single choice.
**Options**: (a) `keyring` crate — talks to OS keychain directly (macOS Keychain, Windows Credential Manager, Secret Service); (b) `tauri-plugin-stronghold` — IOTA-developed encrypted vault file with its own snapshot/migration model.
**Decision**: `keyring`.
**Reasoning**: Native OS keychain is the lowest-risk store: no extra vault file to manage, automatic per-user isolation, no extra migration story when the app schema changes. Stronghold's snapshot model adds a moving part that buys us nothing on a single-user desktop. Keyring's API is also straightforwardly mockable for tests.
**Reversibility**: moderate — switching later means re-prompting users for OAuth.

---

## 2026-05-25 — DB migrations: `refinery`

**Context**: SPEC §4 Phase 2 allowed either `refinery` or hand-rolled migrations.
**Decision**: `refinery` with embedded SQL migration files.
**Reasoning**: Explicit, versioned, runs once at startup, supports `embed_migrations!` so all SQL ships in the binary. Hand-rolling is fine until you need a downgrade story or branch-merge ordering, at which point you've reimplemented refinery.
**Reversibility**: easy — pre-Phase 2 there is no schema yet.

---

## 2026-05-25 — Unsubscribe action deferred

**Context**: SPEC §1 mentions "unsubscribe" as a proposed action but no phase covers it.
**Decision**: Park as Phase 6.5 (List-Unsubscribe header parsing + RFC 8058 one-click) in `BACKLOG.md`. Do not implement in Phase 6 unless the user explicitly re-scopes.
**Reasoning**: Unsubscribe touches outbound HTTP to third-party endpoints, requires careful CSRF/content-type handling, and is orthogonal to the Gmail/Graph mutation surface. Better to ship clean archive/trash first.
**Reversibility**: easy.

---

## 2026-05-25 — Classifier confidence semantics

**Context**: SPEC §4 Phase 4 doesn't define confidence.
**Decision**: Require classifier JSON to include `confidence: number ∈ [0,1]`. Confidence < 0.6 maps the cluster to `unknown` for review-queue purposes (threshold tunable).
**Reasoning**: Without a numeric confidence we can't gate auto-actions against low-trust classifications. Hard cutoff at 0.6 is a starting point; will tune in Phase 4 with real data and record the tuned value here.
**Reversibility**: easy.

---

## 2026-05-25 — Model storage on disk: plaintext under app data

**Context**: Where to store nomic-embed + gemma GGUF files.
**Decision**: `<app_data>/chhanni/models/` (macOS: `~/Library/Application Support/chhanni/models/`), plaintext, relying on per-user filesystem permissions.
**Reasoning**: Model weights are not secrets. Encryption at rest of multi-GB tensor files adds startup latency and no real threat-model benefit (anyone with read access to the user's home directory has already won).
**Reversibility**: easy.

---

## 2026-05-25 — SQLite path & encryption

**Context**: Where to put the local DB, and whether to encrypt it.
**Decision**: `<app_data>/chhanni/chhanni.sqlite`, plaintext, filesystem-permission only. SQLCipher tracked in `BACKLOG.md`.
**Reasoning**: Email headers + snippets are sensitive but not at the "compelled disclosure" level we'd need SQLCipher for. Adding SQLCipher means a custom rusqlite build and key derivation UX. Defer unless threat model changes.
**Reversibility**: moderate (one-way migration from plaintext to encrypted).

---

## 2026-05-25 — Phase 0 stack pins

**Context**: Concrete dependency versions for the Phase 0 scaffold.
**Decision**:
- React 18.3 (not 19) — Tauri 2 templates and most plugins are still validated against 18.
- Vite 6, Tailwind v4 (CSS-first via `@tailwindcss/vite`).
- Tauri 2.x, `tauri-plugin-opener` 2 (needed for OAuth browser handoff in Phase 1).
- Rust toolchain "stable" (1.94 in current env), `edition = "2021"`, workspace `rust-version = "1.80"` floor.
- ESLint 9 flat config + `typescript-eslint` `recommendedTypeChecked` rule set.
**Reasoning**: All pins are at-or-near current stable. The React 18 vs 19 choice is the only one with real downside; revisit when Tauri's React 19 stories settle.
**Reversibility**: moderate.

---

## 2026-05-25 — Workspace clippy lints deny `unwrap_used`/`expect_used`/`panic`

**Context**: CLAUDE.md forbids `unwrap()` and `expect()` outside tests.
**Decision**: Encode that rule in `Cargo.toml` `[workspace.lints.clippy]` so `cargo clippy -- -D warnings` enforces it automatically.
**Reasoning**: Mechanical enforcement is cheaper than human review.
**Reversibility**: easy.

---

## 2026-05-25 — OAuth account index in keychain (no SQLite yet)

**Context**: Phase 1 needs to enumerate connected accounts on app restart, but Phase 2 hasn't shipped SQLite. Two options: a small JSON file in app data, or a keychain entry holding the account index.
**Decision**: Single keychain entry `accounts_index` holds a JSON array of `AccountRecord`s. Individual tokens live under `account::<id>::token`.
**Reasoning**: The account list is non-secret but also tiny and already paired with the keychain-resident tokens; co-locating avoids a second persistence boundary that we'd just have to delete in Phase 2.
**Reversibility**: easy — migrate to SQLite in Phase 2 by reading the index once and dropping the keychain entry.

---

## 2026-05-25 — OAuth callback strategy: loopback localhost only

**Context**: Google's "Desktop application" client type supports loopback redirects and "out-of-band" (deprecated). Tauri also supports custom URL schemes via the `deep-link` plugin.
**Decision**: Loopback only (`http://127.0.0.1:<dynamic-port>/callback`).
**Reasoning**: Works for any provider that accepts loopback (Google, Microsoft Graph for native apps). Avoids registering a custom URL scheme with the OS — one fewer install-time hook to coordinate. Dynamic port comes from `TcpListener::bind("127.0.0.1:0")`.
**Reversibility**: moderate — could co-exist with a deep-link backup later.

---

## 2026-05-25 — Token refresh: explicit short retry with backoff

**Context**: CLAUDE.md mandates timeouts and bounded retries on every external call.
**Decision**: `ensure_fresh_token` retries up to 3 times with 250 ms / 500 ms / 1 s exponential backoff. Reqwest client has a 30 s timeout. Callback timeout is 5 minutes.
**Reasoning**: Refresh failures are usually transient (network hiccup, brief 5xx). 3 attempts gives ~2 s of headroom without leaving the UI hanging. 5 min callback is generous enough that users can fish out a 2FA app without rage-quitting.
**Reversibility**: easy.

---

## 2026-05-25 — SQLite connection model: single `Arc<Mutex<Connection>>`

**Context**: Phase 2 needs concurrent access to SQLite from the sync engine, the command layer, and (later) the embedding pipeline. Options: r2d2/deadpool-sqlite pool, sqlx async, or one shared connection behind a Mutex.
**Decision**: One connection wrapped in `Arc<Mutex<Connection>>` via `db::Db`.
**Reasoning**: SQLite serializes writers anyway; with WAL we get cheap concurrent reads but only one writer. Our workload is one mailbox at a time. A pool adds operational complexity (lifetime management, deadlock surface) without a real throughput win at this scale. If the embedding pipeline (Phase 3) ever needs parallel reads, we can add a read-only pool then.
**Reversibility**: easy — `Db` is the only seam.

---

## 2026-05-25 — Sync cursor model: phase + cursor + history_id

**Context**: Gmail's incremental story is two-stage: `messages.list` for the initial walk (paginated, no history baseline), then `history.list(startHistoryId=…)` for incremental updates. We need to model the transition without losing progress on interruption.
**Decision**: `sync_state` row per account holds `phase` ∈ {`initial`, `incremental`}, `cursor_token` (Gmail pageToken during initial), `last_history_id`. After every page we upsert this row inside the same conceptual operation as the message inserts. Transition to `incremental` happens via `mark_initial_complete(account_id, highest_history_id)` once the initial walk's `nextPageToken` is `None`.
**Reasoning**: Single source of truth for "where am I?", trivially resumable, also a useful UI signal ("syncing initial …" vs "checking for changes …"). Storing the highest seen historyId during the initial walk gives `history.list` a valid baseline the moment the walk completes.
**Reversibility**: moderate — schema change would require a migration.

---

## 2026-05-25 — Metadata fetch: parallel `messages.get` not `batchGet`

**Context**: The plan called for "messages.batchGet" to grab 100 metadata payloads per call. Google's REST API does not actually expose a body-form batchGet for `users.messages`; the supported batch mechanism is HTTP multipart against `/batch/gmail/v1`, which complicates retries and per-request token refresh.
**Decision**: Use `FuturesUnordered` with concurrency 20 against `users.messages.get?format=METADATA&metadataHeaders=From,Subject,List-Unsubscribe`.
**Reasoning**: 20 concurrent requests against a single user account is well below per-user quota (~2.5 quota units × 250 calls/page = 50k/min limit). Simpler retry surface, simpler per-request token refresh. Multipart batch optimization is on the BACKLOG if we ever miss the 90s budget.
**Reversibility**: easy.

---

## 2026-05-25 — Snippet + subject stored plaintext in SQLite

**Context**: Subjects and snippets contain PII. Two options: store as-is, store hashed.
**Decision**: Store plaintext. SQLite file is per-user under platform app-data, filesystem-permission only.
**Reasoning**: Clustering + classification need to read these. Hashing them defeats their purpose. SQLCipher is on the BACKLOG if/when the threat model changes.
**Reversibility**: moderate (would require backfill).

---

## 2026-05-25 — llama.cpp delivery: download prebuilt on first run

**Context**: Phase 3 needs `llama-server` on the user's machine. Three options surveyed; user picked download-on-first-run.
**Decision**: Pin a llama.cpp release tag in `sidecar::release::LLAMA_RELEASE_TAG`. On first run, fetch the per-platform archive into `<app_data>/chhanni/bin/<tag>/`, extract, then spawn with `--port 0 --host 127.0.0.1 --embeddings`. Phase 3 ships the downloader + lifecycle scaffolding; archive extraction lands in Phase 3.5 (BACKLOG).
**Reasoning**: ~30 MB installer instead of ~200 MB; no C++ toolchain on dev machines; ships model upgrades trivially. Trade-off: needs network on first run (acceptable for a cloud-LLM-free desktop app — the connection happens once).
**Reversibility**: easy (could switch to vendor+build at any time).

---

## 2026-05-25 — Embedding storage: plain BLOB, no sqlite-vec

**Context**: Plan called for `sqlite-vec` to index embeddings.
**Decision**: Store embeddings as little-endian f32 BLOBs in `embeddings(embedding BLOB)`. Brute-force cosine in Rust for clustering.
**Reasoning**: At 5K-20K vectors × 768 dims × 4 bytes ≈ 15-60 MB the whole index fits in L3 and an unindexed cosine sweep is microseconds. `sqlite-vec` requires a custom rusqlite build + load_extension dance for tens of microseconds of speedup at our scale. Trade-off: if we ever cluster across multiple accounts at 10⁵+ messages we'll feel it.
**Reversibility**: easy — `EmbeddingsRepo` is the only seam.

---

## 2026-05-25 — Clustering algorithm: sender bucket → greedy cosine agglomerative

**Context**: SPEC §4 Phase 3 said "GROUP BY sender, then within sender group cosine > 0.85".
**Decision**: Two-stage exactly as specced. Within-bucket pass is a single greedy walk with a running-mean centroid per cluster — no quadratic pairwise, no DBSCAN. Buckets below `min_split_size=4` short-circuit to a single cluster.
**Reasoning**: Sender is the dominant signal; within-bucket variation is what we use the embedding for. Greedy + running centroid is O(N × K) where K is small (typically 1-3 sub-clusters per sender). Small buckets aren't worth splitting — keeps the cluster count tidy.
**Reversibility**: easy.

---

## 2026-05-25 — Bump model pins to current SOTA (user request)

**Context**: User asked "use the latest models as much as possible" between Phase 3 and Phase 4. Web-checked current state.
**Decision**:
- llama.cpp release tag → `b9310` (today's release, 2026-05-25)
- Embedding model → `nomic-embed-text-v2-moe.Q8_0.gguf` (was v1.5). 512 MB GGUF; MoE; 768-dim Matryoshka output; multilingual; 8192-token context.
- Classifier model → `Qwen3-4B-Instruct-2507-Q4_K_M.gguf` (was Gemma 3 4B IT). ~2.5 GB; July 2025 release; current SOTA in the 4B instruction-tuned class.
**Reasoning**: Qwen3-4B beats Gemma 3 4B on JSON-schema following benchmarks (we lean on this heavily in Phase 4); multilingual is a genuine bonus for mailboxes that mix English with the user's native language; the size envelope is the same. Nomic v2-MoE doubles down on Nomic v1.5's strengths (Matryoshka, long context) while adding multilingual coverage and slightly better MTEB scores at the same on-disk size. All choices remain env-overridable per asset.
**Reversibility**: easy — `release.rs` is the only seam.

---

## 2026-05-25 — Classification: `/completion` with `json_schema`, not `/v1/chat/completions`

**Context**: Two ways to constrain decoding in llama.cpp: GBNF grammar via `/completion` `grammar` field, or a JSON-schema field on the same endpoint (added in late 2024). OpenAI-compatible `/v1/chat/completions` also exists but has more shape variance across versions.
**Decision**: Stay on `/completion` with `prompt` + `json_schema`. Set `cache_prompt: true` and `temperature: 0.0`.
**Reasoning**: `prompt` gives us full control over formatting (no chat template surprises across model families). `json_schema` is conceptually cleaner than hand-rolled GBNF and is what every modern llama.cpp build supports. `cache_prompt` is essential: many clusters share the long system-y prefix; KV-cache reuse is ~5-10× on real hardware.
**Reversibility**: easy.

---

## 2026-05-25 — Classification idempotence via cluster signature

**Context**: Re-running classification should be a no-op unless something materially changed.
**Decision**: A cluster's "signature" is `SHA-256(cluster_key || '|' || sorted_member_ids)`. Stored alongside each classification. We re-classify only when `(signature, model_version, prompt_version)` differs from what's persisted.
**Reasoning**: Membership change → new signature → re-classify. Sample text changes (subject edits, snippet refreshes from a re-sync) are absorbed because the cluster's _membership_ didn't change. Model/prompt bumps invalidate everything cleanly. Cheap to compute and tiny on disk.
**Reversibility**: easy.

---

## 2026-05-25 — Icons: placeholder for now

**Context**: Tauri's `generate_context!` requires icon paths to exist at compile time.
**Decision**: Generate valid zinc-colored PNGs (32, 128, 256) and stub `.icns`/`.ico` files good enough to pass compile-time validation. Real icon design queued in `BACKLOG.md`.
**Reasoning**: Visual design is out of scope this early. Stub files unblock the toolchain without committing to a brand.
**Reversibility**: easy.
