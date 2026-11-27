# CareLibre Server — agent notes

CareLibre Server is the Rust and PostgreSQL backend for CareLibre. It exposes the
REST API used by the desktop and mobile clients and includes the server admin UI.
See `README.md` and `CONTRIBUTING.md` for the full overview and conventions.

## Ways of working

- **Never commit code changes directly to `main`.** Create a branch and open a
  pull request. This includes Rust sources, tests, migrations and build or
  dependency config.
- **Metadata-only changes may go straight to `main`.** Docs, README edits,
  release notes, screenshots, issue templates and this file are examples.
- **Work in a worktree, not a shared checkout.** When starting work on a branch,
  or when the primary checkout is already on another branch, create a dedicated
  `git worktree`. This lets other sessions keep different branches checked out
  simultaneously instead of waiting for one checkout to become free. Put
  worktrees outside the repository, for example
  `../CareLibre-Server-worktrees/<branch-slug>`.
- **Delete branches once their PR is merged or deliberately closed.** Remove the
  worktree, then delete the local branch and the remote branch. Never delete a
  branch with an open PR.
- Do not commit unrelated changes, generated files or another person's work in
  progress. Check `git status` before staging.
- Do not rewrite or discard uncommitted changes unless explicitly asked.
- CI runs only for releases. Run the local checks before pushing or opening a
  pull request; GitHub Actions is the release gate, not the everyday feedback
  loop.
- Follow `CONTRIBUTING.md` for commit style: imperative mood, one logical change
  per commit. Pull requests should list the checks run.

## Branches and worktrees

Create a worktree when you start a branch:

```sh
git fetch origin
git worktree add ../CareLibre-Server-worktrees/fix-roster \
  -b fix/roster origin/main
cd ../CareLibre-Server-worktrees/fix-roster
```

When the pull request is merged or deliberately closed:

```sh
git worktree remove ../CareLibre-Server-worktrees/fix-roster
git branch -d fix/roster
git push origin --delete fix/roster
```

GitHub may already delete the remote branch when a PR merges; delete it yourself
if it remains. `git branch -d` refuses to delete an unmerged branch, which is a
useful safety net. If the PR was squash- or rebase-merged, `-d` may still refuse
because the original commits are not ancestors; confirm the PR is merged before
using `-D`. Use `git worktree remove --force` only when the work is intentionally
being discarded.

## Commands

```sh
make setup                    # .env from .env.example
make fmt                      # format
cargo fmt --all -- --check    # verify formatting without changing files
make lint                     # clippy, warnings as errors
make test                     # unit and integration tests
make test-db                  # schema integration tests (needs PostgreSQL)
cargo test --workspace
```

If you changed a migration, run it against a real PostgreSQL instance and confirm
it is forward-only and safe to re-run. Never edit an applied migration.

## Conventions

`CONTRIBUTING.md` is the source of truth. In particular:

- Errors go through `AppError` and the `{"error": {...}}` envelope.
- Tenant scope comes from `AuthContext`, never from a request body or query.
- Migrations are numbered and forward-only; care data tables include
  `organisation_id` and an index that starts with it.
- OpenAPI annotations stay accurate for every new handler.
- Secrets never enter the repository.

## Releases

Before tagging a release, add `docs/release-notes/vX.Y.Z.md` using
`docs/release-notes/TEMPLATE.md`. The release workflow refuses to publish without
the `New`, `Changed` and `Fixed` sections.

Write each bullet for a care provider, not a developer: describe what they can
now do, what works differently, or what no longer goes wrong. Avoid commit
hashes, file names, function names, API details and implementation jargon.
