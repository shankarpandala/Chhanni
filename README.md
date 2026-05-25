# Chhanni

**On-device inbox cleanup.** Connects to Gmail and Outlook, classifies your mail with a local LLM, lets you review proposed cleanups, executes them in bulk, and offers per-action undo. Email content never leaves your machine.

## Why

Inboxes accumulate years of newsletters, promotional sweeps, transactional receipts, and notifications. The cleanup task is well-defined but tedious. Cloud LLMs can do it but require sending your mail to a third party. Chhanni does the same work entirely on-device using `llama.cpp` + open-weight models.

## What's in the box

| Layer | Tech |
| --- | --- |
| Shell | Tauri 2 |
| Frontend | React 18 + TypeScript (strict) + Vite + Tailwind v4 |
| State | Zustand + TanStack Query |
| Backend | Rust (stable), Tokio, `tracing` |
| Storage | SQLite via `rusqlite` (WAL), `refinery` migrations |
| Inference | `llama.cpp` server, brokered by Rust |
| Embedding model | `Qwen3-Embedding-0.6B` Q8_0 (639 MB) |
| Classifier model | `Qwen3-30B-A3B-Instruct-2507` Q4_K_M (18.6 GB MoE; ~3 B active per token) |
| OAuth | `oauth2` crate (PKCE, loopback redirect) |
| Secrets | OS keychain via the `keyring` crate |

Target dev machine: MacBook M5 Pro, 24 GB. The classifier and embedding model together fit in ~19.25 GB and leave ~5 GB headroom for the OS, browser, and Tauri runtime.

## Architecture in one paragraph

The Rust core owns OAuth, the email APIs, SQLite, the sidecar lifecycle, action execution, and the audit log. The bundled `llama-server` sidecar owns model loading, embeddings, and completions over HTTP on `127.0.0.1`. The React frontend owns nothing persistent — it talks to Rust only through Tauri commands and never makes a network call directly. No frontend → provider path; no frontend → sidecar path.

## Prerequisites

### macOS (target / production)

- macOS 13 or newer on Apple Silicon
- [Xcode Command Line Tools](https://developer.apple.com/xcode/resources/) (`xcode-select --install`)
- [Rust](https://rustup.rs) stable (`rustup default stable`; minimum `1.80`)
- [Node.js 22](https://nodejs.org) (use `nvm install 22 && nvm use 22`)
- [pnpm](https://pnpm.io) 11+ (`npm install -g pnpm`)
- ~25 GB free disk for models + sidecar + build artifacts

### Linux (development)

The Tauri shell needs the system WebKit + GTK stack:

```bash
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev \
  libjavascriptcoregtk-4.1-dev libayatana-appindicator3-dev \
  librsvg2-dev pkg-config
```

### Windows

Standard Rust + Node toolchains. Tauri's [Windows prerequisites](https://tauri.app/start/prerequisites/) (the WebView2 runtime is preinstalled on Windows 11).

## OAuth setup

You need an OAuth client for each provider you want to use. Both are free.

### Gmail (Google Cloud)

1. Open [Google Cloud Console → APIs & Services → Credentials](https://console.cloud.google.com/apis/credentials).
2. Enable the **Gmail API** for your project.
3. Create OAuth credentials of type **Desktop application**.
4. Copy the **Client ID** (and **Client secret** if Google issued one).
5. Add the redirect URI **`http://127.0.0.1`** to the client (Chhanni picks an ephemeral port at runtime).

Required scopes (Chhanni requests these — no manual configuration needed):
- `https://www.googleapis.com/auth/gmail.readonly` (initial sync)
- `https://www.googleapis.com/auth/gmail.modify` (action execution — granted at first action)
- `openid email`

### Outlook (Microsoft Entra / Azure AD)

1. Open [Entra ID → App registrations → New registration](https://entra.microsoft.com).
2. Account types: **Personal Microsoft accounts and accounts in any organizational directory**.
3. Add a **Mobile and desktop** redirect URI of `http://localhost` (Microsoft accepts any loopback port at runtime).
4. Under **API permissions → Add a permission → Microsoft Graph → Delegated**, request:
   - `Mail.ReadWrite`
   - `offline_access`
   - `User.Read`
   - `openid email profile`
5. Copy the **Application (client) ID**.

Public Desktop apps do **not** need a client secret. Leave `GRAPH_CLIENT_SECRET` blank.

## Configure secrets

Copy `.env.example` to `.env` at the repo root and fill in the values:

```bash
cp .env.example .env
```

```dotenv
GMAIL_CLIENT_ID=your-google-client-id.apps.googleusercontent.com
GMAIL_CLIENT_SECRET=optional-google-client-secret

GRAPH_CLIENT_ID=00000000-0000-0000-0000-000000000000
# GRAPH_CLIENT_SECRET intentionally left blank (public client)
```

Chhanni reads these from the parent shell's environment. There is no `.env` autoloader yet — export them before launching:

```bash
export $(grep -v '^#' .env | xargs)
```

## Build

From the repo root:

```bash
pnpm install                     # JS deps
( cd src-tauri && cargo fetch )  # Rust deps (optional; cargo will fetch lazily)
```

### Run the dev shell

```bash
pnpm tauri dev
```

The first launch takes a minute or two while Rust compiles. A native window titled "Chhanni" opens.

### Run the production build

```bash
pnpm tauri build
```

Bundles a signed (if signing is configured) `.app` / `.dmg` on macOS, `.deb` / `.AppImage` on Linux, `.msi` on Windows. Outputs land in `src-tauri/target/release/bundle/`.

## Set up the local LLM sidecar

Phase 3.5 (auto-extraction of the bundled binary) isn't wired up yet, so for now you run the sidecar yourself in two terminals. This is a one-time step — Chhanni connects to whatever ports you supply.

### 1. Install `llama-server`

```bash
# macOS via Homebrew
brew install llama.cpp

# Or download a prebuilt binary for tag b9310+ from
# https://github.com/ggml-org/llama.cpp/releases
```

### 2. Download the models

Pick a directory and download both GGUFs (~19 GB total):

```bash
mkdir -p ~/Library/Application\ Support/com.chhanni.chhanni/models
cd ~/Library/Application\ Support/com.chhanni.chhanni/models

# Embedding model (639 MB)
curl -L -O https://huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF/resolve/main/Qwen3-Embedding-0.6B-Q8_0.gguf

# Classifier model (18.6 GB — this will take a while)
curl -L -O https://huggingface.co/unsloth/Qwen3-30B-A3B-Instruct-2507-GGUF/resolve/main/Qwen3-30B-A3B-Instruct-2507-Q4_K_M.gguf
```

### 3. Launch two sidecar processes

In one terminal — the embedding server on port 8080:

```bash
llama-server \
  -m ~/Library/Application\ Support/com.chhanni.chhanni/models/Qwen3-Embedding-0.6B-Q8_0.gguf \
  --host 127.0.0.1 --port 8080 --embeddings
```

In another terminal — the classifier on port 8081 (Apple Silicon defaults to Metal):

```bash
llama-server \
  -m ~/Library/Application\ Support/com.chhanni.chhanni/models/Qwen3-30B-A3B-Instruct-2507-Q4_K_M.gguf \
  --host 127.0.0.1 --port 8081 \
  -ngl 999 -c 8192
```

`-ngl 999` puts all layers on the GPU, `-c 8192` is the context window. On a 24 GB machine the 30B-A3B model uses ~18 GB of unified memory.

Both processes log a "listening on http://127.0.0.1:PORT" line once they're ready (~20-30 s for the classifier).

## Using the app

1. **Connect an account.** Open the *Accounts* tab. Click "Connect Gmail" or "Connect Outlook". Complete the consent flow in your browser. The account card appears.
2. **Sync.** Click *Sync*. Headers and snippets for every message are pulled into the local SQLite DB. Resumable: kill the app mid-sync and restart, it picks up from the last checkpoint. Idempotent re-runs.
3. **Embed.** Enter the embedding sidecar's port (8080 by default), click *Embed*. Every message gets a 1024-dim Qwen3 vector. Idempotent — re-runs only embed messages that don't yet have a vector.
4. **Cluster.** Click *Cluster*. Messages are grouped first by sender, then by cosine similarity > 0.85 within each sender group. The cluster panel shows a sample row per cluster.
5. **Classify.** Enter the classifier sidecar's port (8081), click *Classify*. Each cluster is sent to Qwen3-30B-A3B with a JSON-schema constraint and gets one of nine categories (transactional, newsletter, social, personal, work, security, promotional, notification, unknown) with a confidence score. Sub-0.6 confidence collapses to unknown.
6. **Review.** Switch to the *Review* tab. The default filter shows clusters with proposed actions:
   - `promotional + age > 90d` → propose **archive**
   - `notification + ≥ 50 members` → propose **trash**
   - `newsletter + List-Unsubscribe` → propose **unsubscribe**
   - `transactional + > 1 year` → propose **archive**
   - `security` / `personal` / `work` → never propose destructive actions
7. **Stage and run.** Click an action chip on a cluster to stage it. The chip flips to "✓ <action>". When you're happy, click *Run cleanup*. Approved actions execute against the provider in 1000-message batches with exponential backoff. Cancel mid-run; partially-applied batches log a `cancelled` outcome.
8. **Audit + undo.** Switch to the *History* tab. Every executed action has a row. Click *Undo* to reverse a single action. Trash undos work within 30 days. Label changes restore the prior label set. Export the full audit log as CSV or JSON.

## Tests + gates

```bash
( cd src-tauri && cargo check )
( cd src-tauri && cargo clippy --all-targets -- -D warnings )
( cd src-tauri && cargo test )
pnpm typecheck
pnpm build
pnpm lint
```

All six must pass before any phase is considered done. The full suite has **119 Rust tests** at time of writing.

`scripts/check.sh` runs the Rust + frontend gates in sequence.

## Storage layout

| What | Where (macOS) |
| --- | --- |
| SQLite DB | `~/Library/Application Support/com.chhanni.chhanni/chhanni.sqlite` |
| Models | `~/Library/Application Support/com.chhanni.chhanni/models/` |
| Tokens | OS keychain, service `chhanni` |
| App logs | stdout / `tracing` (level via `RUST_LOG=chhanni=debug`) |

Override the data dir for local testing:

```bash
export CHHANNI_DATA_DIR=/tmp/chhanni
```

## Privacy guarantees

- **No cloud LLM calls. Ever.** All inference is the local `llama.cpp` sidecar.
- **No PII in logs.** Subjects, bodies, addresses, tokens are never logged. Opaque IDs only.
- **Secrets only in the OS keychain.** Greps for `Bearer`, `client_secret`, or `refresh_token` outside the `auth/` module return nothing.
- **No frontend → provider API path.** The frontend talks only to Tauri commands. Rust brokers everything.
- **No frontend → sidecar path.** Inference is brokered by Rust so we can audit and rate-limit.

See `DECISIONS.md` for the full reasoning behind every non-trivial choice.

## Project documentation

- `SPEC.md` — product spec
- `PLAN.md` — phase-by-phase plan with acceptance criteria
- `PROGRESS.md` — append-only build log
- `DECISIONS.md` — append-only decisions log
- `BACKLOG.md` — out-of-scope items deliberately deferred
- `CLAUDE.md` — conventions for future contributors

## License

Proprietary; see `Cargo.toml`.
