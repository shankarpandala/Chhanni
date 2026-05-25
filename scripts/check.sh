#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

echo "==> cargo check"
(cd src-tauri && cargo check --locked)

echo "==> cargo clippy"
(cd src-tauri && cargo clippy --locked --all-targets -- -D warnings)

echo "==> cargo test"
(cd src-tauri && cargo test --locked --quiet)

echo "==> pnpm typecheck"
pnpm typecheck

echo "==> pnpm build"
pnpm build

echo "All gates passed."
