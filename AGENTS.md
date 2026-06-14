# Agent context — E-R Bridge - Geodesic

Project context, stack, routing, auth, and DB conventions: see [`openspec/AGENTS.md`](openspec/AGENTS.md).

Stack-specific coding conventions live in skills:

- Backend (Rust/Axum): `.claude/skills/rust-rest-api/SKILL.md`

Spec-driven workflow: see `openspec/` (proposals, specs, tasks under `openspec/changes/`).

## Commit conventions

This repo uses [Conventional Commits](https://www.conventionalcommits.org/). Every
commit subject MUST be `type(scope)?: description`, lower-case, imperative, no
trailing period. Common types: `feat`, `fix`, `docs`, `refactor`, `test`,
`chore`, `ci`, `build`, `perf`. Breaking changes use a `!` (e.g. `feat!:`) and a
`BREAKING CHANGE:` footer. See [`CONTRIBUTING.md`](CONTRIBUTING.md) for details.
