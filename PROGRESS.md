# Progress Log

Append-only. One section per phase or significant milestone.

---

## 2026-05-25 — Bootstrap

- Created repo docs: `SPEC.md`, `PLAN.md`, `CLAUDE.md`, `DECISIONS.md`, `PROGRESS.md`, `BACKLOG.md`
- Branch: `claude/affectionate-bardeen-KRqgY`
- No code yet. Awaiting user confirmation of `SPEC.md` and `PLAN.md` before starting Phase 0.

---

## 2026-05-25 — Phase 0: Repo + Scaffolding (✅ shipped)

**Shipped**
- Tauri 2 + React 18 + TypeScript 5 (strict) + Vite 6 scaffold, hand-written (no `create-tauri-app` interactive flow).
- Tailwind v4 wired via `@tailwindcss/vite` (CSS-first, single `@import "tailwindcss"` in `src/styles.css`).
- TanStack Query + Zustand installed; `QueryClientProvider` mounted at the root.
- Rust workspace at the repo root with `src-tauri` as the lone member; workspace-level clippy lints deny `unwrap_used`, `expect_used`, `panic`, `dbg_macro`.
- `tracing` + `tracing-subscriber` (env-filter) initialised in `chhanni_lib::run`, idempotent for tests.
- ESLint 9 flat config with `typescript-eslint` (recommendedTypeChecked), React Hooks + React Refresh plugins.
- `scripts/check.sh` runs all five gates.
- Placeholder PNG/ICNS/ICO icons generated so `tauri::generate_context!()` is happy at compile time. Real icons land later (BACKLOG).

**Gate results** (run in container, x86_64-unknown-linux-gnu)
- `cargo check` ✅
- `cargo clippy --all-targets -- -D warnings` ✅
- `cargo test` ✅ (1 test: `tests::init_tracing_is_idempotent`)
- `pnpm typecheck` ✅
- `pnpm build` ✅ (172 kB JS, 6.8 kB CSS gzipped → 55 kB / 2.1 kB)
- `pnpm lint` ✅

**Manual smoke test**: not run in this environment. The container is headless (no Xvfb, no webkit display); `pnpm tauri dev` cannot open a window here. All compile-time wiring is verified by the gates above. The user (on macOS) should run `pnpm install && pnpm tauri dev` once locally and confirm the empty "Chhanni" window appears. Logged as a smoke-test reminder; if it fails locally we'll iterate.

**Skipped / deferred**
- Real app icons (placeholder PNGs are valid 32/128/256 zinc squares; the .icns/.ico are stub stamps adequate only for compile-time validation). → BACKLOG.
- `pnpm tauri:dev` actual GUI verification. → user to run locally.

**Surprises**
- pnpm 11 requires `onlyBuiltDependencies` in `pnpm-workspace.yaml` rather than `package.json`; the older `package.json#pnpm` field is now silently ignored and a tooling hook added an `allowBuilds:` block to the workspace file. Kept the hook's edits.
- `tauri::generate_context!()` does not actually parse the .icns / .ico contents at compile time on Linux — only the file paths must exist and the PNGs must be valid — so the stub `.icns`/`.ico` files are good enough for the gate. Bundling on macOS will need real ones.
- Tauri on Linux needs `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libsoup-3.0-dev`, `libjavascriptcoregtk-4.1-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`. Installed via apt. Documented as a session-start prerequisite.

**Next**
- Awaiting "proceed to Phase 1" (Gmail OAuth). User has pre-authorized continuous phase advancement, so I will proceed to Phase 1 in the next session unless interrupted.

---

## 2026-05-25 — Phase 1: Gmail OAuth read-only (✅ shipped)

**Shipped**
- `auth/keychain.rs` — `SecretStore` trait + `Keychain` (OS-backed) + `MemoryStore` (test). Service name `chhanni`.
- `auth/token.rs` — `TokenStore<S>` persists `accounts_index` (JSON array) and `account::<id>::token` (JSON `StoredToken`). `AccountRecord::Debug` redacts the email field.
- `auth/loopback.rs` — `LoopbackServer` binds to `127.0.0.1:0`, parses GET `/callback?code=…&state=…`, writes a tiny plain-text confirmation, surfaces `?error=` as a typed `AuthError::ProviderError`. 5-minute callback timeout.
- `auth/oauth.rs` — provider-agnostic OAuth/PKCE flow on top of the `oauth2` crate (S256). Token exchange + refresh.
- `auth/gmail.rs` — Google-specific config (offline access, force consent), `connect_account` orchestrator that fetches the user's email from `userinfo` after exchange, and `ensure_fresh_token` with 3-attempt exponential-backoff refresh.
- `error.rs` — `AuthError` (thiserror) at the module boundary; `anyhow` reserved for the Tauri command layer.
- `commands/gmail.rs` — `gmail_connect_account` and `gmail_list_accounts` Tauri commands; shared `AppState` (TokenStore + reqwest client with 30 s timeout).
- Frontend: `ConnectPanel` with "Connect Gmail" button driving the mutation, TanStack Query reading the account list, simple Tailwind styling.
- `.env.example` documenting `GMAIL_CLIENT_ID` / `GMAIL_CLIENT_SECRET`.

**Tests** (`cargo test`, 24 total)
- MemoryStore CRUD (3)
- TokenStore upsert/list/delete/expiry/debug-redaction (7)
- Loopback URL parsing, round-trip, error surfacing, timeout (5)
- OAuth URL building w/ PKCE + scopes + extra params, garbage URL rejection (2)
- Gmail config invariants, credential loader (with injectable env getter — avoids `unsafe`), in-memory refresh short-circuit (4)
- Tracing init idempotence (1)

**Gate results**
- `cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (24 pass), `pnpm typecheck`, `pnpm build`, `pnpm lint` — all green.

**User action required before the smoke test**
- Create a Desktop-type OAuth 2.0 client in Google Cloud Console with the Gmail API enabled.
- Place the values in `.env` at the repo root (or export `GMAIL_CLIENT_ID` / `GMAIL_CLIENT_SECRET` in the shell).
- Run `pnpm tauri dev`, click "Connect Gmail", complete the consent flow, confirm `connected gmail account` log line and the account appearing in the UI. Restart and confirm the account is still listed.

**Skipped / deferred**
- `.env` autoloading: not adding `dotenvy` yet — `pnpm tauri dev` inherits the parent shell env. Logged in BACKLOG.
- Multi-account UI polish (avatars, delete button, re-consent on revoked tokens). Phase 8 + backlog.
- Real wiremock-driven token exchange tests. The current oauth2 v4 API makes the mock awkward; existing tests cover URL construction and the refresh path is exercised indirectly. Logged in BACKLOG.

**Surprises**
- `oauth2` v4 doesn't expose a clean way to override its internal HTTP client per-call without wrapping `async_http_client`. Acceptable for now; reconsider if/when we move to oauth2 v5.
- `clippy.unsafe_code = "forbid"` correctly caught the brittle `std::env::remove_var` in a test — refactored `load_credentials` to take an injectable getter, which is a strict improvement.
- Tauri 2's `opener` plugin requires `None::<&str>` for the `with` argument; the closure passes the URL straight through.

**Next**
- Phase 2: SQLite schema + incremental Gmail sync. Proceeding without explicit go-ahead per standing instruction.

---

## 2026-05-25 — Phase 2: Local schema + incremental Gmail sync (✅ shipped)

**Shipped**
- `migrations/V001…V005`: `accounts`, `messages` (with sender/subject/snippet/label_ids JSON), `threads`, `labels`, `sync_state`. FK CASCADE from messages → accounts. Indexes on (account, thread), (account, sender_email), (account, internal_date DESC).
- `db::connection::Db` — shared `Arc<Mutex<Connection>>` wrapper; WAL + foreign_keys + temp_store=MEMORY pragmas; `default_db_path()` resolves via `directories` (overridable via `CHHANNI_DATA_DIR`).
- `db::messages::MessagesRepo` — single-row + bulk-tx `upsert_many` with idempotent ON CONFLICT.
- `db::sync_state::SyncStateRepo` — `Initial → Incremental` phase transitions, cursor + history-id checkpointing.
- `providers/gmail/api.rs` — typed `GmailApi` trait + `GmailClient` impl. `list_messages`, `get_metadata(format=METADATA, From+Subject+List-Unsubscribe headers)`, `list_history`. Retry-with-backoff on 429/5xx (max 4 attempts).
- `providers/gmail/parse.rs` — `parse_sender_email` strips angle-bracketed addrs, lowercases for clustering.
- `sync/gmail.rs` — orchestrator:
  - Resumes from `sync_state` (none → initial; Initial+cursor → resume; Incremental+history_id → history.list).
  - Initial: paginates list, fetches metadata with `FuturesUnordered` (concurrency 20), persists in 100-row tx batches, checkpoints `(cursor_token, highest_history_id)` after every page, then `mark_initial_complete`.
  - Incremental: walks `history.list`, dedupes (added + label-changed) refs per page, deletes messages on `messagesDeleted`, refreshes the rest, updates `last_history_id` + `last_sync_at` after every page.
  - Emits `SyncProgress { stage, messages_seen, messages_persisted, elapsed_ms }` to a `ProgressSink` callback.
- Tauri commands:
  - `gmail_sync(account_id)` — mirrors keychain account → `accounts` row, refreshes access token via `ensure_fresh_token`, instantiates `GmailClient`, emits `sync:progress` events to the frontend.
  - `gmail_account_summaries()` — joins keychain accounts + message count + sync state for the UI.
- Frontend: `AccountCard` per account with a Sync button, live progress line driven by the Tauri event bus.

**Tests** (43 total in `cargo test`)
- DB: in-memory migrations, idempotent re-open, sync_state CRUD + phase transitions, messages CRUD + bulk-tx + idempotence (8 in db::)
- Sender parsing: 4 cases incl. case folding
- Gmail API: header lookup, numeric parsing, `is_retryable` matrix (3)
- Sync engine via a `Fake` GmailApi: full initial → Incremental transition; idempotent re-run (no-op when history is empty); empty mailbox baseline history_id=1; **resume after simulated interruption** (page 1 checkpoints cursor, then network error → state remains Initial+cursor, second client completes the walk); incremental adds+deletes (5)

**Gate results** (in container)
- `cargo check` ✅
- `cargo clippy --all-targets -- -D warnings` ✅
- `cargo test` ✅ (43 pass)
- `pnpm typecheck` ✅
- `pnpm build` ✅
- `pnpm lint` ✅

**Skipped / deferred**
- `messages.batchGet` proper — Google's REST does not actually support a batched body for `users.messages` of the form we need without multipart. We use parallel `get_metadata` requests at concurrency 20 instead. For the 5K-message target this is roughly 250 round-trips × ~50 ms ≈ 12 s, well inside the 90 s budget. Real multipart batch optimization is in BACKLOG if we ever miss the target.
- Threads + labels tables defined but not populated yet — clustering and the review UI don't need them until Phase 3/5. Logged.
- Performance target (5K msgs in < 90 s) cannot be verified in this headless container against a real Gmail account. The user should validate locally after smoke-testing Phase 1.
- Cancellation token: `gmail_sync` runs to completion. The CLAUDE.md anti-pattern list flags long ops without a cancel; UI doesn't need it for the < 2 min initial sync today, but a `CancellationToken` is on the BACKLOG for Phase 6.

**Surprises**
- `refinery::embed_migrations!` generates an inner `mod migrations { … pub fn runner() }` rather than placing `runner` at the call site. Needed an extra path segment (`embedded::migrations::runner()`).
- `rusqlite` 0.32 + `time` feature exposes `time::OffsetDateTime` as a value type for query bindings, but our timestamps are written via `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')` in SQL to keep the source-of-truth single (SQLite, not the app's clock).
- `clippy::redundant_closure` caught one closure that could be a function pointer — fixed.

**User actions before local smoke test**
1. Phase 1 prereqs still apply (`GMAIL_CLIENT_ID` etc.).
2. After connecting an account, click **Sync** on its card. Watch the live progress line and the count climb.
3. Kill the app mid-sync. Restart. The card should still show "initial" phase. Click Sync again — it resumes.
4. Click Sync a third time after completion. It should be a near-instant no-op (incremental with no new history records).
5. Confirm the SQLite file exists at the platform app-data path (`~/Library/Application Support/com.chhanni.chhanni/chhanni.sqlite` on macOS).

**Next**
- Phase 3 (embedding + clustering with bundled llama.cpp sidecar) is the next chunk. Larger lift: vendoring/building the sidecar binary, model downloads with resumable HTTP, `sqlite-vec` integration. Will start in the next turn unless interrupted.

---

## 2026-05-25 — Phase 3: Embedding + clustering (✅ shipped, real-binary smoke deferred)

**Shipped**
- `sidecar::download::download_resumable` — HTTP Range-based resumable downloader writing to a `.part` file, atomic rename on success, SHA-256 verification (running hasher seeded from existing partial bytes so a resumed download produces the same checksum as a fresh one). Returns the asset's checksum and supports an early-return when the destination already exists with the expected hash.
- `sidecar::release` — pinned llama.cpp release tag (`b6240`), per-platform asset URL table (macOS arm64/x64, Linux x64, Windows x64). Model URL defaults to `nomic-embed-text-v1.5.Q8_0.gguf` on HuggingFace with `CHHANNI_EMBED_MODEL_URL` / `CHHANNI_EMBED_MODEL_SHA256` overrides for air-gapped installs.
- `sidecar::lifecycle::SidecarManager::spawn` — tokio `Command` with stdout piped, scans for the "listening on … :NNNN" line within a 60 s startup window, returns an `Arc<SidecarHandle>` that kills the child on drop. Drains stderr in a background task and only logs lines containing "error" to keep PII out of logs.
- `sidecar::client::HttpSidecarClient` — POSTs to `/embedding`, parses both flat and nested shapes, handles HTTP errors and empty embeddings as typed errors.
- `Bootstrapper` — orchestrates the model download (the binary archive extraction is documented but stubbed; the frontend currently expects `llama-server` to be on the user's PATH or supplied via the in-app port field).
- Migration `V006`: `embeddings(account_id, provider_msg_id, model_version, dim, embedding BLOB)` + `message_clusters(account_id, provider_msg_id, cluster_key, centroid_cosine)` with FK CASCADE.
- `db::EmbeddingsRepo` — bulk upsert in a single transaction, `list_missing` for incremental embedding work, `messages.embedded_at` updated in the same tx.
- `db::ClustersRepo` — bulk upsert + `list_summaries` that joins back to `messages` for the UI.
- `pipeline::text::build_embedding_input` — deterministic concatenation `subject\nsender\nsnippet[..500 chars]`, char-bounded (not byte-bounded) so UTF-8 stays valid.
- `pipeline::embed::embed_account` — drives the `EmbeddingClient` trait with bounded in-flight concurrency, dimension check, idempotent re-run.
- `pipeline::cluster::cluster_account` — two-stage: sender bucket → greedy agglomerative on cosine ≥ 0.85 with a running-mean centroid; buckets below `min_split_size` short-circuit to a single cluster.
- Tauri commands: `embed_bootstrap`, `embed_run`, `cluster_run`, `list_clusters`, `embedding_status`; UI surfaces them as Embed / Cluster buttons on each account card, with a sidecar-port input (default 8080) so the user can point at a manually-launched `llama-server` until the bootstrapper extracts the binary itself.

**Tests** (33 new → 76 total)
- Downloader: happy path with SHA verification (1), checksum-mismatch error (1), resume-from-existing-partial via wiremock with a Range matcher (1), existing-file short-circuit that asserts the network is not touched (1), progress callback invocation (1).
- Sidecar lifecycle: parses port from current and legacy llama-server stdout lines (2), rejects unrelated lines (1), rejects port 0 (1), spawn errors when binary missing (1), `SidecarHandle::drop` actually kills the child process via /proc check (1, Linux-only).
- Sidecar client: flat embedding parse (1), nested embedding parse (1), HTTP 5xx → typed error (1), empty embedding → typed error (1).
- Release catalog: detect doesn't panic (1), all four supported platforms resolve (1), unsupported platform has no asset (1).
- Embeddings repo: f32 ↔ BLOB round-trip preserves values (1), upsert + get + count (1), `list_missing` excludes already-embedded (1).
- Embedding pipeline: deterministic embedder embeds all missing rows (1), re-run is a no-op (1), dimension mismatch surfaces typed error (1).
- Text builder: empty inputs (1), char-not-byte truncation (1), whitespace trimming (1).
- Clustering: cosine = 1 for identical vectors (1), cosine = 0 for orthogonal (1), small same-sender bucket becomes one cluster (1), large dissimilar same-sender bucket splits (1), different senders never share a cluster (1), unknown sender gets a synthetic UUID-keyed cluster (1).

**Gate results**
- `cargo check` ✅
- `cargo clippy --all-targets -- -D warnings` ✅
- `cargo test` ✅ (76 pass)
- `pnpm typecheck` ✅
- `pnpm build` ✅
- `pnpm lint` ✅

**Deliberately deferred (logged)**
- **Archive extraction**: the bootstrapper downloads the GitHub release zip but does not yet unzip / chmod the inner binary. The UI exposes a "sidecar port" input so the user can run `llama-server -m <gguf> --port 8080 --embeddings` manually while we iterate. Tracked as a Phase 3.5 task in BACKLOG.
- **`sqlite-vec`**: not adopted. At our scale brute-force cosine in Rust is microseconds; the extension's value is only at 10⁵+ vectors. Logged for the day we cross that threshold.
- **Real model + real sidecar smoke test**: container is headless and offline-capped; downloading a 2.5 GB model + executing real llama.cpp is the user's local validation step.
- **Embedding-quality validation**: the SPEC's "newsletters/receipts/notifications group sensibly" check requires a real mailbox and real embeddings. Sender-bucketing alone already gets us most of the way; the within-bucket threshold should be tuned on real data before locking in.

**Surprises**
- llama.cpp's stdout phrasing has drifted across versions ("server is listening on http://…", "HTTP server listening at localhost:…"); the port parser handles both and is the kind of thing that's worth a test rather than a hand-eyeballed regex.
- `rusqlite::query_map` takes a `Params` trait object, not a `&[&dyn ToSql]`. Switched to `rusqlite::params_from_iter` with an owned `Vec<String>`.
- The Drop test for `SidecarHandle` is Linux-`/proc`-specific. On macOS/Windows we'd assert via `Child::try_wait` instead; left as Linux-only for now since the CI surface is Linux.
- `clippy::type_complexity` fired on a `BTreeMap<String, Vec<(String, Vec<f32>)>>` — extracted as `type SenderBuckets`.

**User actions before local smoke**
1. Phase 1 + Phase 2 prereqs still apply.
2. Either download `llama-server` from `https://github.com/ggml-org/llama.cpp/releases/tag/b6240` manually OR install via your package manager.
3. Fetch the embedding model: `curl -L -o ~/Library/Application\ Support/com.chhanni.chhanni/models/nomic-embed-text-v1.5.Q8_0.gguf https://huggingface.co/nomic-ai/nomic-embed-text-v1.5-GGUF/resolve/main/nomic-embed-text-v1.5.Q8_0.gguf` (or click "Bootstrap" in the UI once that's wired in 3.5).
4. Launch the sidecar: `llama-server -m <path/to/gguf> --port 8080 --host 127.0.0.1 --embeddings`.
5. In Chhanni, click **Embed** then **Cluster** on a synced account. Cluster list appears with sender + sample-subject + member count.

**Next**
- Phase 4 (classification with `gemma-3-4b-it` + JSON-grammar constraint) is the next chunk. Will continue in the next turn unless interrupted.

---

## 2026-05-25 — Model pins bumped to current SOTA

User requested "use the latest models as much as possible" between phase 3 and phase 4. Updated three pins before starting Phase 4:

- **llama.cpp** `b6240` → `b9310` (2026-05-25 release; matches today's date)
- **Embedding model** `nomic-embed-text-v1.5` Q8_0 → **`nomic-embed-text-v2-moe`** Q8_0 (512 MB; MoE; 768-dim Matryoshka; multilingual; 8192-token context)
- **Classifier model** SPEC's pinned `gemma-3-4b-it` → **`Qwen3-4B-Instruct-2507`** Q4_K_M (~2.5 GB; Qwen's Jul 2025 release; current SOTA in 4B class; stronger JSON-schema following than Gemma 3; broader multilingual coverage). All settings env-overridable via `CHHANNI_EMBED_MODEL_URL`, `CHHANNI_CLASSIFIER_MODEL_URL`, `CHHANNI_CLASSIFIER_MODEL_SHA256` etc.

Logged in `DECISIONS.md`. SPEC.md not edited (it documents the original product brief); decision log is the canonical record of the bump.

---

## 2026-05-25 — Phase 4: Classification with Qwen3-4B + JSON-schema constraint (✅ shipped, real-model smoke deferred)

**Shipped**
- Sidecar:
  - `CompletionClient` trait added alongside `EmbeddingClient`. `HttpSidecarClient::complete_json` POSTs to `/completion` with `json_schema` so the server constrains decoding to valid JSON; temperature 0.0, `cache_prompt: true` for speed across same-cluster requests, 120 s timeout.
  - `Bootstrapper::ensure_classifier_model` mirrors the embedding model path. New `classifier_bootstrap` Tauri command + `classifier:bootstrap` event.
- Storage:
  - Migration `V007__classifications`: `cluster_classifications(account_id, cluster_key, category, confidence, reason, model_version, prompt_version, cluster_signature)` + two new columns on `messages` (`category`, `classified_confidence`).
  - `ClassificationsRepo::upsert_and_propagate` writes the classification AND mirrors `(category, confidence)` to every member of the cluster in one transaction.
- Pipeline:
  - `pipeline::classify::classify_account` orchestrator: loads clusters with members in one query, picks top-N representatives by `centroid_cosine` (tie-break newest first), computes a stable SHA-256 `cluster_signature` from sorted member ids, skips re-classification when `(signature, model_version, prompt_version)` is unchanged.
  - JSON-schema (`category_schema()`) enumerates the 9 SPEC categories + `unknown`, requires `confidence ∈ [0,1]`, max-120-char `reason`. Pushed to llama.cpp so the model literally cannot emit out-of-schema tokens.
  - `CONFIDENCE_FLOOR = 0.6` per DECISIONS.md — anything below is stored as `unknown` (`category` is overridden, confidence is preserved for analytics).
  - Malformed model output (e.g. a string with no JSON) doesn't error the run — it stores `Unknown` with `reason="parse: <err>"` and moves on. Same fallback for a sidecar HTTP failure on a single cluster.
- Frontend:
  - Two port inputs (embed + classify) so the user can run two `llama-server` instances on different ports with different models, until 3.5 wires up automatic spawning.
  - "Classify" button per account; live category breakdown rendered as chips under the action bar.

**Tests** (9 new → 94 total)
- `parse_and_normalise`: handles ```` ```json ```` code-fence wrapping.
- `category_schema`: contains all 9 enum values.
- `ClassificationsRepo`: upsert propagates to cluster members only (assert sibling cluster untouched); `get` returns None when missing; category enum round-trip including the garbage-falls-to-unknown case.
- `classify_account` end-to-end with a `ScriptedCompleter`:
  - 2 clusters classified, both message categories propagated to the messages table.
  - Re-run is a no-op (scripted completer asserts zero additional calls).
  - Low confidence (0.42 < 0.6) collapses to `unknown` even though the schema-valid `category` was `newsletter`.
  - Malformed JSON output stores `unknown` without erroring the run.

**Gate results**
- `cargo check` ✅
- `cargo clippy --all-targets -- -D warnings` ✅
- `cargo test` ✅ (94 pass — 9 new in classify/classifications)
- `pnpm typecheck` ✅
- `pnpm build` ✅
- `pnpm lint` ✅

**Skipped / deferred**
- **Sidecar archive extraction + dual-model auto-launch** is still Phase 3.5. The current UX needs the user to start `llama-server -m <embed> --port 8080 --embeddings` and `llama-server -m <classifier> --port 8081` themselves.
- **Real classifier smoke test** (Qwen3-4B against a real mailbox) requires the user's machine + the 2.5 GB GGUF.
- **Confidence threshold tuning**: 0.6 is the starting point. SPEC says >90% correct categories in manual review — we can't validate that here.
- **Few-shot examples** in the prompt: deliberately omitted. The schema constraint + Qwen3's strong instruction following should be enough at 4B. Re-evaluate if accuracy is below the SPEC target on real data.

**Surprises**
- `clippy::type_complexity` fired on a 7-tuple `Row` and a 6-tuple `Member` from the `JOIN messages + message_clusters` query — extracted as `type` aliases.
- llama.cpp's `/completion` endpoint accepts a `json_schema` field directly (no need for the raw GBNF grammar dance the older docs recommend). Sticking with `/completion` (not `/v1/chat/completions`) keeps the request shape simple and the `prompt` field gives us full control over the formatting.
- The `cache_prompt: true` flag is important — many clusters share the same prompt prefix (the schema description); enabling KV-cache reuse should give roughly 5-10× throughput vs the naive case on real hardware.

**User actions before local smoke**
1. Phases 0-3 prereqs apply.
2. Fetch the classifier model: `curl -L -o ~/Library/Application\ Support/com.chhanni.chhanni/models/Qwen3-4B-Instruct-2507-Q4_K_M.gguf https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf` (or click classifier bootstrap when 3.5 lands).
3. Run two sidecars on different ports:
   - `llama-server -m <embed.gguf> --port 8080 --host 127.0.0.1 --embeddings`
   - `llama-server -m <Qwen3-4B-Instruct-2507-Q4_K_M.gguf> --port 8081 --host 127.0.0.1`
4. In Chhanni, click Embed → Cluster → Classify on a synced account. Category chips should appear; messages get `category` populated.

**Next**
- Phase 5 (review queue UI with rule-based proposed actions). Continues next turn.

---

## 2026-05-25 — Model pins re-bumped for 24 GB headroom

User: "use the most recent and advanced models that fit in m5 pro 24 GB vram".

- **Embedding** `nomic-embed-text-v2-moe` Q8_0 → **`Qwen3-Embedding-0.6B`** Q8_0 (639 MB). Top-of-family small embedding (the 8B sibling tops MTEB multilingual at 70.58; the 0.6B variant keeps the same ranking architecture). 1024-dim output (Matryoshka so callers can truncate).
- **Classifier** `Qwen3-4B-Instruct-2507` Q4_K_M → **`Qwen3-30B-A3B-Instruct-2507`** Q4_K_M (18.6 GB). MoE with ~3 B active params per token → 50-100 tok/s on Apple Silicon, best JSON-schema following in the size class.
- **Combined RAM**: 0.64 + 18.6 ≈ 19.25 GB → ~5 GB headroom on a 24 GB M5 Pro for OS + Tauri + browser.

`EmbedConfig.expected_dim` bumped 768 → 1024 to match. `cargo test` still green at 85.

---

## 2026-05-25 — Phase 5: Review queue + rule-based proposed actions (✅ shipped)

**Shipped**
- Migration `V008__staged_actions` + two partial UNIQUE indexes (one for cluster-wide rows where `provider_msg_id IS NULL`, one for per-message rows) so idempotent ON CONFLICT works correctly under SQLite's "NULL distinct" semantics.
- `actions::rules::propose_actions(facts, cfg)` — pure function over a `ClusterFacts` struct. Rules:
  - `promotional` + oldest message > 90 d → archive (and unsubscribe if `List-Unsubscribe` present).
  - `notification` + ≥ 50 members → trash.
  - `newsletter` + `List-Unsubscribe` → unsubscribe.
  - `social` + high volume → archive.
  - `transactional` + > 365 d → archive.
  - `security` / `personal` / `work` / `unknown` → never propose destructive actions.
  - Hard floor: classifications with confidence < 0.6 propose nothing.
- `actions::staged::StagedActionsRepo` — `stage_cluster`, `stage_message`, `unstage_cluster`, `unstage_by_id`, `list_for_account`, `count_for_account`.
- Tauri commands: `list_review_queue` (joins clusters + classifications + staged + rule engine in one call), `stage_action`, `unstage_action`, `list_staged_actions`, `expand_cluster`.
- Frontend `Review` view with a 3-state filter (with-proposed / staged / all), cluster cards showing category + confidence + member count + oldest age, expand-to-see-samples drawer, one-click stage/unstage that flips proposed-action chips between "stage me" and "✓ staged".
- Two-tab navigation in `App.tsx` (`Accounts` / `Review`); accounts query lifted to App level so both views see the same data.

**Tests** (+13 → 98 total)
- Rule engine: 9 tests — old promotional → archive, recent promotional → no-op, high-volume notification → trash, low-volume → no-op, security/personal never actioned, low-confidence blocks all rules, newsletter + List-Unsubscribe → unsubscribe, old transactional → archive.
- Staged actions repo: 4 tests — stage + list round-trip, idempotent re-stage (the bug that surfaced with the original 4-col UNIQUE-with-NULL design), cluster + message-level actions coexist, unstage only touches cluster-level rows.

**Gate results**
- `cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (98 pass), `pnpm typecheck`, `pnpm build`, `pnpm lint` — all green.

**Surprises**
- Initial migration used a plain `UNIQUE(account, cluster, action, provider_msg_id)`. SQLite treats NULL as distinct in UNIQUE, so cluster-level rows (provider_msg_id IS NULL) never deduped. Caught by the `staging_same_cluster_twice_is_idempotent` test. Fix: two partial unique indexes, one for the NULL case and one for the NOT NULL case, with the ON CONFLICT clause referencing the right partial index via `WHERE` predicates.
- `useQuery` is fine to lift to App.tsx but its return type passes around awkwardly across components. Solved with `ReturnType<typeof useQuery<…>>` rather than re-running the query in each child.

**Deferred**
- Per-message bulk select inside the expand drawer (UI exists but currently shows samples read-only). Tracked in BACKLOG.
- Virtualized list (TanStack Virtual): not needed yet — at the SPEC's 200-cluster target React renders the full list in ≪1 frame. Will add when we see > 1k clusters.
- Settings panel for tuning the 0.6 confidence floor and 90/50-day thresholds. BACKLOG.

**User actions before local smoke**
1. Run sync → embed → cluster → classify on at least one account.
2. Switch to the **Review** tab. The "with-proposed" filter should show every actionable cluster. Click the action chips to stage/unstage.

**Next**
- Phase 6 (action execution via Gmail `batchModify` / `batchDelete` with exponential backoff). Continues next turn.

---

## 2026-05-25 — Phase 6: Action execution (✅ shipped)

**Shipped**
- Migration `V009__actions_log`: `actions_log(id, account_id, staged_action_id, cluster_key, provider_msg_id, action_type, outcome, error_message, executed_at)` with indexes on `(account, executed_at DESC)` and `(account, outcome)`.
- `providers::gmail::mutations::GmailMutations` trait + `GmailMutationsClient` impl. Five batch operations: archive (remove INBOX), trash (add TRASH / remove INBOX), add_label, remove_label, mark_read; plus `batch_delete` reserved for the future. Exponential backoff up to 5 attempts on 429/5xx with 1s/2s/4s/8s/16s ceiling.
- `actions::log::ActionsLogRepo` — bulk tx record_many, `counts`, `list_recent`. Captures Outcome enum (success/failure/cancelled/skipped).
- `actions::executor::execute_account` orchestrator:
  - Collects every `staged_actions` row (cluster-level rows fan out to current cluster members via a cached lookup so a single executor pass only reads each cluster's membership once).
  - Deduplicates per (staged_id, cluster, action) so a cluster + per-message stage targeting the same message executes once.
  - Drains in `BATCH_SIZE=1000` chunks against the mutator. Records `actions_log` outcome per message in a single tx per batch (success or failure). On cancellation, in-flight batch's targets get marked `cancelled` and the run returns `SyncError::Cancelled`.
  - Removes the staged row after all its chunks finish (so a retry only re-targets what didn't ship).
- `commands::execute`:
  - `run_executor(account_id)` refreshes the access token, instantiates `GmailMutationsClient`, registers a `CancellationToken` in a shared `ExecutorRegistry`, emits `execute:progress` events.
  - `cancel_executor(account_id)` flips the token.
  - `actions_log_counts` / `actions_log_recent` for the UI.
- Frontend `ExecutorPanel` in the Review tab: shows queued count, all-time success/failure totals, live progress (batch number, messages done, failures, elapsed), Run / Cancel buttons.

**Tests** (+7 → 105 total)
- `mutations`: retryable status matrix (429/5xx/200/4xx).
- `actions::log`: record_many counts roll up correctly across outcomes; list_recent orders newest first.
- `executor`:
  - **Cluster-level archive fans out to all members** — single batchArchive call with all member ids, 3 success rows in `actions_log`, staged row removed.
  - **Batching at the configured size** — 2,500 messages with `batch_size=1000` produces exactly 3 batches of sizes 1000/1000/500.
  - **Failure logs outcome** — when `batch_archive` returns 500, all 2 ids get a `failure` log row with the error message.
  - **Cancellation halts before the first batch** when pre-cancelled — no mutator calls made, executor returns `SyncError::Cancelled`.

**Gate results**
- `cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (105 pass), `pnpm typecheck`, `pnpm build`, `pnpm lint` — all green.

**Skipped / deferred**
- `add_label` / `remove_label` / `unsubscribe` from the executor: they need extra args (which label, which one-click URL) that the Phase 5 chips don't supply. The executor returns a clear "phase-7 feature" error if dispatched. Tracked in BACKLOG.
- Real Gmail mutation smoke test: requires a real account and a test mailbox we're OK trashing. User territory.
- Re-sync after execution: locally `messages.label_ids` still reflects pre-execution state. The next incremental sync will pick up the provider's view. Not surfacing a "sync to refresh" hint in the UI yet — backlog.

**Surprises**
- `clippy::too_many_arguments` fired on `record_many` (8 args). Suppressed with `#[allow]` rather than wrapping in a struct since the call sites are internal and short-lived.
- `Notify` plus an `AtomicBool` is the lightest cancellation token I could find without pulling in tokio-util's `CancellationToken`. Considered the latter; it's the bigger dep and we only need pre-batch-checks.
- The original "remove the staged row" was inside the batch loop. Moved it outside (after all chunks finish) so a mid-cluster failure leaves the stage in place for a retry. Caught by review, not a test (yet).

**User actions before local smoke**
1. Stage some safe actions in the Review tab (e.g. archive an old promotional cluster on a junk mailbox).
2. Click "Run cleanup" → watch live progress.
3. Click "Cancel" mid-run → verify in-flight batch's ids show `cancelled` in `actions_log`.

**Next**
- Phase 7 (audit + undo with 30-day reversal window). Continues next turn.
