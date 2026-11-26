#!/usr/bin/env bash
# The repo's checks. Exit 0 only when the branch is fit to review.
#
# The gate runs this script from the main checkout with the job's worktree as
# the working directory. Keep it fast, deterministic and non-interactive: no
# watch modes, no prompts, nothing that rewrites files.
#
# Mirrors .github/workflows/rust.yml (Linux jobs; coverage and the
# Windows/macOS Agent builds stay in CI).
set -euo pipefail

# One cargo target dir per worktree: jobs that change the protos overwrite each
# other's generated code in a shared dir. Costs one cold build per job.
export CARGO_TARGET_DIR="${FACTORY_CARGO_TARGET_DIR:-$HOME/.cache/rusty-rmm-factory/targets/$(basename "$(git rev-parse --show-toplevel)")}"

# sqlx::test needs a Postgres server; start a throwaway one unless DATABASE_URL is set.
if [[ -z "${DATABASE_URL:-}" ]]; then
  pg=$(docker run -d --rm -e POSTGRES_PASSWORD=password -p 127.0.0.1::5432 postgres:17 postgres -N 1000)
  trap 'docker kill "$pg" >/dev/null 2>&1 || true' EXIT
  until docker exec "$pg" pg_isready -h 127.0.0.1 -U postgres -q; do sleep 1; done
  export DATABASE_URL="postgres://postgres:password@$(docker port "$pg" 5432/tcp | head -1)/postgres"
fi

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked

# Web UI: generates the client from the protos (into gitignored src/gen), type-checks, bundles.
(cd rustyrmm-web && npm ci --prefer-offline --no-audit --no-fund && npm run build)
