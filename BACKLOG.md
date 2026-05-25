# Backlog

Things discovered out of current phase scope. Do not silently implement.

- **One-click unsubscribe (RFC 8058)**: SPEC §1 lists unsubscribe as an action but no phase explicitly covers parsing `List-Unsubscribe` headers and POSTing the one-click endpoint. Land alongside Phase 6 or as Phase 6.5.
- **Cross-platform support**: Windows + Linux. Out of scope; tracked here so we don't accidentally over-portabilize early code.
- **Token re-consent flow**: when Google revokes / scopes change, surface a re-connect UX. Not in Phase 1.
- **Cluster threshold tuning UI**: expose the 0.85 cosine threshold and rule parameters in a settings panel.
- **Embeddings re-index on prompt/model version bump**: define migration strategy.
- **End-to-end encryption at rest for SQLite**: currently relying on filesystem permissions. Evaluate SQLCipher if threat model requires it.
- **Telemetry / crash reporting**: must be local-first if added at all. Currently zero telemetry by design.
