#!/bin/bash
# Local CI. Run this before every commit; it is the only CI this project has.
#   scripts/ci.sh          format check, clippy, tests, release build
#   scripts/ci.sh --fix    apply rustfmt first, then run everything
set -euo pipefail
cd "$(dirname "$0")/.."

step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }

if [ "${1:-}" = "--fix" ]; then
  step "cargo fmt"
  cargo fmt
fi

step "cargo fmt --check"
cargo fmt --check

step "cargo clippy"
cargo clippy --release --all-targets -- -D warnings

step "cargo test"
cargo test --release

step "cargo build --release"
cargo build --release

printf '\n\033[32mAll checks passed.\033[0m\n'
