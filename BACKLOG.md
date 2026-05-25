# Backlog

Things discovered out of current phase scope. Do not silently implement.

- **One-click unsubscribe (RFC 8058)**: SPEC §1 lists unsubscribe as an action but no phase explicitly covers parsing `List-Unsubscribe` headers and POSTing the one-click endpoint. Land alongside Phase 6 or as Phase 6.5.
- **Cross-platform support**: Windows + Linux. Out of scope; tracked here so we don't accidentally over-portabilize early code.
- **Token re-consent flow**: when Google revokes / scopes change, surface a re-connect UX. Not in Phase 1.
- **Cluster threshold tuning UI**: expose the 0.85 cosine threshold and rule parameters in a settings panel.
- **Embeddings re-index on prompt/model version bump**: define migration strategy.
- **End-to-end encryption at rest for SQLite**: currently relying on filesystem permissions. Evaluate SQLCipher if threat model requires it.
- **Telemetry / crash reporting**: must be local-first if added at all. Currently zero telemetry by design.
- **Real app icons**: current `src-tauri/icons/*` are zinc-colored placeholders + stub .icns/.ico files. Replace before any release build.
- **Linux dev prerequisites doc**: `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev pkg-config` must be installed for `cargo check` to pass in container. Worth scripting via a session-start hook.
- **SQLCipher** for encrypted-at-rest SQLite. Tracking against a future threat-model bump.
- **`dotenvy` for local dev**: load `.env` automatically on `pnpm tauri dev` so contributors don't have to export shell vars.
- **End-to-end OAuth tests via `wiremock`**: simulate Google's token endpoint to cover the full code-for-token swap and refresh path.
- **Account-management UX**: remove button, re-consent on revoked refresh tokens, avatars from `userinfo.picture`.
- **Gmail multipart batch endpoint** at `/batch/gmail/v1` for metadata fetches if parallel `get` ever misses the 90 s budget on the target hardware.
- **Sync cancellation token** wired through `gmail_sync` and surfaced as a cancel button.
- **Threads + labels population** during sync (tables exist but unused).
- **Sync scheduling** (background re-sync every N minutes, not just on-click).
- **SQLite vacuum / WAL truncation** policy after large delete batches in Phase 6.
- **Phase 3.5 — sidecar archive extraction**: the bootstrapper downloads the llama.cpp release zip; we still need to unzip + chmod + verify the inner binary, then launch it via `SidecarManager` automatically. Until then the UI exposes a port input.
- **sqlite-vec** for ≥10⁵ messages or cross-account search.
- **Embedding threshold tuner**: surface the 0.85 cosine threshold in a settings panel for real-mailbox tuning.
- **Re-embed migrations** when `model_version` changes.
- **Few-shot examples in the classifier prompt** if real-world accuracy is below the SPEC's 90 % bar.
- **Per-language category routing** — the multilingual embedding/classifier choice opens this door; defer until we see a non-English mailbox in the wild.
- **Stop using two `llama-server` processes**: one process can load multiple models since b8xxx; revisit when 3.5 lands.
- **Per-message bulk select in the cluster expand drawer** (currently read-only samples).
- **Virtualized review list** when cluster count exceeds 1k.
- **Settings panel** for the rule thresholds (90 d, 50 members, 0.6 confidence floor) — design needed.
