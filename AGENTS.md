# RustyRMM

A Rust workspace (`rustyrmm-server`, `rustyrmm-agent`, `rustyrmm-cli`,
`rustyrmm-proto`) plus a Preact web UI in `rustyrmm-web`. Domain terms are in
`CONTEXT.md`; deployment is in `doc/deploy.md`.

## Setup

A fresh checkout needs a Rust stable toolchain (with rustfmt and clippy),
Node 24 and Docker. Then:

```bash
(cd rustyrmm-web && npm ci)
```

Server tests use `sqlx::test`, which needs a Postgres server in `DATABASE_URL`
(for example `postgres://postgres:password@localhost:5432/postgres`).
`scripts/init_db.sh` starts one in Docker. No env files to copy.

## Dev server

The web UI alone, on a given port:

```bash
cd rustyrmm-web && npm run dev -- --port <port> --strictPort
```

It shows the sign-in page; anything past that needs a running Server
(`rustyrmm-server serve`, which needs Postgres and a TLS certificate; see
`doc/deploy.md`).

## Validation

`factory/checks.sh` runs all of these (and starts a throwaway Postgres if
`DATABASE_URL` is unset):

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
(cd rustyrmm-web && npm ci && npm run build)
```

## Tunnels

An Operator opens a Tunnel from the CLI to a port on an Endpoint's loopback. Opening
needs a permission, off by default; any Operator can grant it:

```bash
rustyrmm operator tunnels <username> --allow          # or --deny
rustyrmm tunnel open <endpoint> <port> [--listen-port N] [--bind ADDR]  # until Ctrl-C
rustyrmm tunnel list                                  # your Tunnels
rustyrmm tunnel close <tunnel-id>
```

`tunnel open` listens on `127.0.0.1` (OS-chosen port unless `--listen-port`) and prints the
Tunnel id and address. The Tunnel ends with the CLI process, `tunnel close`, or the Endpoint's
Session.

## Software factory

`/factory run` builds every open GitHub issue labeled `factory` through
intake → spec → build → gate → review → approve, one PR per issue. State is in
`factory/jobs/` (gitignored); `factory/bin/job` is its only writer. Checks:
`factory/checks.sh`. Dashboard: `cd factory && python3 -m http.server`, then
open http://localhost:8000/dashboard.html.

Each job works in its own git worktree, a full checkout of the repo at
`.worktrees/<job-id>` (gitignored). A tool run from the repo root that doesn't
read `.gitignore` sees those copies too; exclude `.worktrees/` in its config.
