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
