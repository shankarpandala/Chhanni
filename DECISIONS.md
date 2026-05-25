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

## 2026-05-25 — Icons: placeholder for now

**Context**: Tauri's `generate_context!` requires icon paths to exist at compile time.
**Decision**: Generate valid zinc-colored PNGs (32, 128, 256) and stub `.icns`/`.ico` files good enough to pass compile-time validation. Real icon design queued in `BACKLOG.md`.
**Reasoning**: Visual design is out of scope this early. Stub files unblock the toolchain without committing to a brand.
**Reversibility**: easy.
