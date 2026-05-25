# Chhanni — Conventions for Future Sessions

Read this at the start of every session. Then read `PROGRESS.md` to learn where the last session stopped.

## Privacy first

- **No cloud LLM calls. Ever.** All inference is the bundled `llama.cpp` sidecar on `127.0.0.1`.
- **No PII in logs.** Never log email subjects, bodies, sender addresses, recipient addresses, tokens, or refresh tokens. Log opaque IDs (UUIDs, provider message IDs are OK as identifiers, not as content).
- **Secrets only in OS keychain** via the chosen plugin. Grepping the codebase outside `auth/` for `Bearer`, `client_secret`, `refresh_token` should return nothing.
- **No frontend → provider API path.** The frontend talks only to Tauri commands. Rust brokers everything.
- **No frontend → sidecar path.** Inference is brokered by Rust so we can audit and rate-limit it.

## Rust

- No `unwrap()` or `expect()` outside tests. Use `thiserror` for typed errors within modules. `anyhow` is allowed only at the Tauri command boundary.
- Every external call has a timeout. Every retry policy has a max attempt count.
- `cargo clippy -- -D warnings` must stay clean.
- `tracing` for all logs. Use `info!`, `warn!`, `error!` with structured fields. Log levels controlled via `RUST_LOG`.
- Tests for every module with non-trivial logic.

## TypeScript / Frontend

- `tsconfig.json` strict mode. Zero `any`. Zero `@ts-ignore` without a justification comment on the line above.
- Zustand for ephemeral UI state. TanStack Query for anything fetched from Rust.
- Components are small and typed. Hooks live in `src/lib/hooks/`.

## Commits & branches

- Conventional commits: `feat:`, `fix:`, `chore:`, `refactor:`, `test:`, `docs:`.
- One commit per logical unit.
- Never amend committed work without saying so explicitly in chat.
- Branches: `phase-N-<slug>` for phase work. The current top-level branch in this environment is `claude/affectionate-bardeen-KRqgY` — push there.

## Gates (must all pass before declaring a phase done)

```
cargo check
cargo clippy -- -D warnings
cargo test
pnpm typecheck
pnpm build
```

Plus a manual smoke test of the phase's primary user flow.

## Phase discipline

- Build one phase at a time.
- After each phase: update `PROGRESS.md` (what shipped, what was skipped, what surprised you).
- Update `DECISIONS.md` for every non-trivial choice (library swap, schema design, prompt template, threshold value).
- **Stop. Wait for the user to say "proceed to Phase N+1".**
- Out-of-scope discoveries → `BACKLOG.md`, never silent scope expansion.

## Anti-patterns (stop and revert)

- Cloud LLM call of any kind
- Tokens or email content outside SQLite + keychain
- PII in logs
- Writing to the inbox before Phase 6
- Bulk-fetching full message bodies during initial sync
- Skipping gate checks
- Starting the next phase without explicit go-ahead

## When ambiguous

- Materially design-changing ambiguity → ask **one** focused question, then wait.
- Trivial (variable name, internal helper layout) → pick the conventional option, log it in `DECISIONS.md`, move on.
