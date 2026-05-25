# Chhanni — Specification

## What it is

Chhanni is a desktop application (Tauri 2 shell, React frontend, Rust core) that helps users clean up bloated Gmail and Outlook/Live mailboxes. Its defining property: **email content never leaves the user's machine**. All language-model inference runs locally via a bundled `llama.cpp` sidecar with Metal acceleration on Apple Silicon.

## How it works

1. **Connect**: The user authorizes Chhanni against Gmail (OAuth 2.0 + PKCE, read-only initially) and/or Microsoft Graph. Tokens live in the OS keychain.
2. **Sync**: Chhanni performs an incremental, resumable sync of message headers and snippets into a local SQLite database. Full bodies are never bulk-fetched.
3. **Embed + cluster**: Each message (subject + sender + snippet excerpt) is embedded with `nomic-embed-text-v1.5` into `sqlite-vec`. Messages are grouped by sender, then by cosine similarity > 0.85.
4. **Classify**: Three representative samples per cluster are sent to `gemma-3-4b-it` (also local) with a strict JSON schema. The cluster receives a category — transactional, newsletter, promotional, etc. — with a confidence score, propagated to all members.
5. **Review**: A frontend queue presents proposed actions (archive, label, trash, unsubscribe) derived from rules over the classification. The user approves or rejects, in bulk or individually. Nothing executes until the user pulls the trigger.
6. **Execute**: Approved actions are batched against Gmail/Graph with backoff. Every action is written to an immutable audit log with a reversal payload, enabling undo within the provider's retention window.

## Non-negotiable constraints

- No cloud LLM calls, ever.
- No email subject/body/address in logs.
- No writes to the inbox before Phase 6.
- No frontend → provider API path; the Rust core brokers everything.
- No inference path that bypasses the Rust core; the frontend never speaks to the sidecar directly.

## Target environment

MacBook M5 Pro, 24GB. Cross-platform is out of scope for now; design choices may assume macOS + Apple Silicon (Metal, `~/Library/Application Support`).

## Phasing

Eight phases, strictly sequential, each with a Definition of Done and a hard stop for user approval before the next begins. Phases 0–7 cover Gmail end-to-end; Phase 8 mirrors the read/write surface for Outlook and unifies the queue.

## Open questions / flags

- **Unsubscribe action**: §1 lists "unsubscribe" as a proposed action, but no phase covers parsing `List-Unsubscribe` headers or executing one-click unsubscribe (RFC 8058). I will treat unsubscribe as a Phase 6+ action and add a backlog item if it's not in scope yet.
- **`tauri-plugin-stronghold` vs `keyring` crate**: §2 mentions both. Stronghold is a Tauri plugin with its own encrypted vault; `keyring` talks to the OS keychain directly. These behave differently on restore/migration. I will default to `keyring` (OS-native, simpler, no extra vault to manage) and log the decision in `DECISIONS.md`. Confirm or override.
- **Refinery vs hand-rolled migrations**: §4 Phase 2 lists either. I'll pick `refinery` for clarity and embed migrations at compile time. Logged in `DECISIONS.md`.
- **Sidecar binary distribution**: `llama.cpp` server is not a stable, versioned artifact. I'll pin a specific upstream commit and build it locally as part of the Tauri sidecar bundling step, recorded in `DECISIONS.md`.
- **Cluster threshold (0.85)**: this is a starting point; Phase 3 should expose it as a config so we can tune empirically before locking it in.
- **"Confidence score" semantics**: §4 Phase 4 doesn't define how the model emits confidence. I'll require the JSON schema to include a `confidence: number in [0,1]` field and treat low-confidence clusters as `unknown` for review-queue purposes.
- **First-run model download size**: nomic-embed (~140MB Q8_0) + gemma-3-4b-it (~2.5GB Q4_K_M). Phase 3 needs a progress UI and resumable downloads; flagged so it isn't underestimated.
