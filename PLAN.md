# Chhanni — Phase-by-Phase Plan

Each phase is sequential. Do not start phase N+1 until the user explicitly says "proceed to Phase N+1".

Repo layout (target):

```
chhanni/
├── src/                       # React + TS frontend
│   ├── routes/
│   ├── components/
│   ├── stores/                # Zustand
│   ├── lib/                   # tauri command wrappers, query client
│   └── main.tsx
├── src-tauri/
│   ├── src/
│   │   ├── main.rs
│   │   ├── lib.rs
│   │   ├── commands/          # Tauri command handlers (thin)
│   │   ├── auth/              # OAuth + keychain
│   │   ├── providers/
│   │   │   ├── gmail/
│   │   │   └── graph/
│   │   ├── db/                # rusqlite, migrations
│   │   ├── sync/              # incremental sync engine
│   │   ├── sidecar/           # llama.cpp lifecycle + HTTP client
│   │   ├── pipeline/          # embed → cluster → classify
│   │   ├── actions/           # staging + execution
│   │   ├── audit/             # log + undo
│   │   └── error.rs
│   ├── migrations/
│   ├── binaries/              # bundled llama.cpp server
│   ├── tauri.conf.json
│   └── Cargo.toml
├── package.json
├── pnpm-workspace.yaml
├── vite.config.ts
├── tailwind.config.ts
├── tsconfig.json
├── SPEC.md
├── PLAN.md
├── DECISIONS.md
├── PROGRESS.md
├── BACKLOG.md
└── CLAUDE.md
```

---

## Phase 0 — Repo + Scaffolding

**Goal**: Empty Tauri app that launches and passes all gates.

Tasks:
- [ ] `pnpm create tauri-app` → name `chhanni`, React + TS template, pnpm
- [ ] Upgrade to Tauri 2 stable (CLI + plugins) if template defaults older
- [ ] Add Tailwind v4 (`@tailwindcss/vite` plugin), configure `index.css`
- [ ] `tsconfig.json` → `"strict": true`, `"noUncheckedIndexedAccess": true`
- [ ] ESLint flat config with `@typescript-eslint`, `eslint-plugin-react-hooks`
- [ ] `src-tauri/Cargo.toml` workspace setup; add `tracing`, `tracing-subscriber`, `thiserror`, `anyhow`, `tokio` with required features
- [ ] Initialize `tracing_subscriber` in `main.rs` with `RUST_LOG` env support
- [ ] Add `rustfmt.toml` and `clippy.toml`
- [ ] Create `CLAUDE.md` (conventions from §6)
- [ ] Create stubs for `SPEC.md`, `PLAN.md`, `DECISIONS.md`, `PROGRESS.md`, `BACKLOG.md` (this step)
- [ ] CI-style `scripts/check.sh` running all four gates locally

Acceptance:
- `pnpm tauri dev` launches a window titled "Chhanni"
- `cargo check && cargo clippy -- -D warnings` clean
- `pnpm typecheck && pnpm build` clean
- `tracing` emits a startup log line

---

## Phase 1 — Gmail OAuth (read-only)

**Goal**: User can connect Gmail; token survives restart.

Tasks:
- [ ] Add `oauth2`, `reqwest` (rustls), `keyring`, `url`, `serde`, `serde_json`
- [ ] `auth/gmail.rs`: PKCE flow, loopback redirect (`http://127.0.0.1:<port>/callback`) on a `tiny_http` listener
- [ ] Scope: `https://www.googleapis.com/auth/gmail.readonly`
- [ ] Tauri command `gmail_connect_account()` → opens browser via `tauri-plugin-opener`, listens for callback, exchanges code, stores refresh+access token in keychain under `chhanni::gmail::<account_id>`
- [ ] `auth/token.rs`: refresh helper with retry (max 3, exponential backoff), used by every Gmail API call
- [ ] Frontend: minimal "Connect Gmail" button on a `/connect` route; on success, show connected email
- [ ] Tests: token refresh logic (mock token endpoint), keychain round-trip
- [ ] `.env.example` documenting `GMAIL_CLIENT_ID`, `GMAIL_CLIENT_SECRET` (compiled-in via build script or env, never logged)

Acceptance:
- Click → browser opens → consent → app logs `connected: account_id=<uuid>`
- Restart app, run `gmail_list_accounts()`, account still present, access token refreshes silently

---

## Phase 2 — Local Schema + Incremental Sync

**Goal**: Pull 5K messages (headers/snippets) in <90s, resumable, idempotent.

Tasks:
- [ ] Add `refinery`, `rusqlite` with `bundled` and `serde_json`
- [ ] Migrations:
  - `001_accounts.sql`: `accounts(id, provider, email, created_at)`
  - `002_messages.sql`: `messages(id, account_id, provider_msg_id, thread_id, sender, subject_hash, snippet_hash, internal_date, label_ids JSON, raw_headers JSON, embedded_at, classified_at)`
  - `003_threads.sql`: `threads(id, account_id, provider_thread_id, last_history_id)`
  - `004_labels.sql`: `labels(id, account_id, provider_label_id, name, type)`
  - `005_sync_state.sql`: `sync_state(account_id PRIMARY KEY, last_history_id, last_sync_at, cursor_token)`
- [ ] **Note**: store `subject_hash`/`snippet_hash` for dedupe/diff; full text only in encrypted-at-rest fields if needed (decision pending — default to plain text in SQLite file under app data dir, with note in DECISIONS.md)
- [ ] `providers/gmail/api.rs`: typed client around `users.messages.list`, `users.messages.get(format=METADATA)`, `users.history.list`
- [ ] `sync/gmail.rs`:
  - First sync: paginate `messages.list`, batch 100 metadata fetches via `users.messages.batchGet` (or fall back to parallel `get`)
  - Subsequent sync: `history.list(startHistoryId=last_history_id)`
  - Checkpoint `last_history_id` after every page
- [ ] Tauri command `gmail_sync(account_id)` → progress events via Tauri event bus
- [ ] Tests: mock Gmail API; verify resume after kill mid-sync
- [ ] Frontend: minimal sync progress bar on `/connect`

Acceptance:
- Test mailbox of 5K msgs → sync in <90s on dev hardware
- Kill mid-sync, restart, completes with same final row count
- Second `gmail_sync` immediately after first is a no-op (0 new rows)

---

## Phase 3 — Embedding + Clustering

**Goal**: Local sidecar embeds all messages; sensible clusters appear.

Tasks:
- [ ] Pin `llama.cpp` upstream commit; document in `DECISIONS.md`
- [ ] Build script `src-tauri/build.rs` compiles `llama-server` for the target triple, places binary in `src-tauri/binaries/llama-server-<triple>`
- [ ] `sidecar/lifecycle.rs`: spawn on app start with `--port 0` (let OS pick), parse port from stdout, kill on shutdown signal
- [ ] `sidecar/client.rs`: HTTP client for `/embedding` and `/completion`; timeouts 30s, retries 2
- [ ] Model download:
  - `models/download.rs`: resumable HTTP download (Range requests) to `<app_data>/models/`
  - Verify SHA256
  - First-run UI prompts and shows progress
- [ ] Add `sqlite-vec` extension; migration `006_vec.sql` creates `vec_messages(message_id, embedding FLOAT[768])`
- [ ] `pipeline/embed.rs`: build text = `subject || "\n" || sender || "\n" || snippet[..500]`, embed, store
- [ ] `pipeline/cluster.rs`: GROUP BY sender; within each group, agglomerative cluster on cosine sim > 0.85 (threshold in config); write `cluster_id` back to `messages`
- [ ] Tauri command `list_clusters(account_id)` → `Vec<ClusterSummary>`
- [ ] Tests: embedding determinism (same input → same vector ± epsilon), clustering on synthetic fixtures

Acceptance:
- 5K messages → 50–200 clusters
- Manual spot-check: newsletters/receipts/notifications group sensibly
- End-to-end embed+cluster in <3 min

---

## Phase 4 — Classification

**Goal**: Each cluster gets a category + confidence; persisted, idempotent.

Tasks:
- [ ] Download `gemma-3-4b-it` Q4_K_M GGUF via same mechanism (resumable + sha256)
- [ ] Sidecar reload: support holding both embed + chat models, or switch on demand (decide in DECISIONS.md based on memory budget — likely two sidecar instances on different ports)
- [ ] `pipeline/classify.rs`:
  - Prompt template (versioned in code) requesting strict JSON:
    `{"category": "...", "confidence": 0.0–1.0, "reason": "<≤80 chars>"}`
  - 3 representative samples per cluster (e.g. centroid + two random)
  - JSON-mode / grammar constraint via llama.cpp `json_schema` parameter
  - On parse failure: 1 retry with `"return ONLY valid JSON"` reminder, then mark `unknown`
- [ ] Migration `007_classifications.sql`: `cluster_classifications(cluster_id PK, category, confidence, reason, model_version, prompt_version, created_at)`
- [ ] Propagate to messages: `messages.category` column (materialized for query speed)
- [ ] Skip reclassify if `(cluster_signature, prompt_version, model_version)` unchanged
- [ ] Tests: prompt-rendering golden tests; JSON parser handles trailing whitespace, code fences

Acceptance:
- Manual review on test mailbox: >90% correct category per cluster
- 200 clusters classified in <5 min
- Re-run is a no-op for unchanged clusters

---

## Phase 5 — Review Queue UI

**Goal**: User can browse and stage actions for 200 clusters smoothly.

Tasks:
- [ ] Rule engine (`actions/rules.rs`): pure function `(cluster, messages) -> Vec<ProposedAction>`
  - `promotional` & oldest_message_age > 90d → archive
  - `notification` & member_count > 50 → trash
  - `newsletter` & has_list_unsubscribe → propose unsubscribe (deferred execution)
  - Default → no action
- [ ] Migration `008_staged_actions.sql`: `staged_actions(id, cluster_id, message_id NULL, action_type, payload JSON, staged_at)`
- [ ] Tauri commands: `list_review_queue()`, `stage_action()`, `unstage_action()`, `expand_cluster()`
- [ ] Frontend `/review`:
  - Virtualized list (TanStack Virtual) of clusters grouped by proposed action
  - Expand → sample messages
  - Bulk approve / per-message approve
  - "Staged" sidebar count
- [ ] Zustand store for staging selections; TanStack Query for cluster data
- [ ] Tests: rule engine fixtures; command round-trips

Acceptance:
- 200 clusters render without jank (scroll, expand)
- Staged actions survive restart

---

## Phase 6 — Action Execution

**Goal**: Approved actions execute against Gmail in batches.

Tasks:
- [ ] `providers/gmail/mutations.rs`: `batchModify`, `batchDelete` wrappers
- [ ] `actions/executor.rs`:
  - Drain `staged_actions` in batches of 1000 message IDs
  - Group by action_type
  - Exponential backoff on 429/5xx (1s, 2s, 4s, 8s, max 5)
  - Per-message outcome captured
- [ ] Migration `009_actions_log.sql`: `actions_log(id, staged_action_id, message_id, outcome, error_message, executed_at)`
- [ ] Cancellation: `CancellationToken` (tokio-util) checked between batches
- [ ] Frontend: "Run Cleanup" button → progress modal with cancel
- [ ] Tests: mocked Gmail mutation server; cancellation mid-batch

Acceptance:
- 1000 staged actions execute against test mailbox with correct outcomes
- Cancel button stops further batches; in-flight batch completes cleanly

---

## Phase 7 — Audit + Undo

**Goal**: Every action reversible (within provider retention); exportable log.

Tasks:
- [ ] Migration `010_audit.sql`: `audit_entries(id, action_log_id, before_state JSON, after_state JSON, reversal_payload JSON, timestamp)`
- [ ] On execution success: write audit entry with prior `label_ids` and post-action state
- [ ] `audit/undo.rs`: per action_type reversal
  - Label changes → reapply original labels via batchModify
  - Trash within 30d → untrash via `messages.untrash`
  - Permanent delete → cannot undo, surface error
- [ ] Tauri commands: `list_audit(filter)`, `undo_action(audit_id)`, `export_audit(format)`
- [ ] Frontend: `/history` route with filter + undo button + export
- [ ] Tests: reversal correctness via mock provider

Acceptance:
- Undo any action from current session, verify state restored
- Export produces well-formed CSV and JSON

---

## Phase 8 — Outlook / Microsoft Graph

**Goal**: Parity with Gmail across connect/sync/execute; unified queue.

Tasks:
- [ ] `auth/graph.rs`: MSAL-style PKCE; scope `Mail.ReadWrite`, `offline_access`
- [ ] `providers/graph/api.rs`: list messages with `$select`, deltas via `/messages/delta`
- [ ] `sync/graph.rs`: delta-token-based incremental sync, mirroring Gmail's history flow
- [ ] `providers/graph/mutations.rs`: `move` to archive/deleted-items folders; batch via `$batch` endpoint (max 20)
- [ ] Multi-account UI: account switcher, provider badge in cluster cards
- [ ] Unified review queue: clusters from both providers in one list, filterable
- [ ] Tests: delta sync resume; mutation backoff

Acceptance:
- Connect both Gmail and Outlook in one session
- Sync both; review unified queue; run actions across both providers

---

## Cross-Phase Standing Items

- Update `PROGRESS.md` after each phase
- Update `DECISIONS.md` for every non-trivial choice
- Stop after each phase. Wait for "proceed to Phase N+1".
- Anything out of scope → `BACKLOG.md`, no silent expansion.
