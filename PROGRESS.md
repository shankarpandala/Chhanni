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
