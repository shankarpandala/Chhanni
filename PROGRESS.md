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
