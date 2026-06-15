# Contributing

## Commit messages — Conventional Commits

This repo follows [Conventional Commits](https://www.conventionalcommits.org/).
Each commit subject is:

```
type(scope)?: description
```

- **type** — one of:
  - `feat` — a new feature
  - `fix` — a bug fix
  - `docs` — documentation only
  - `refactor` — a code change that neither fixes a bug nor adds a feature
  - `test` — adding or correcting tests
  - `perf` — a performance improvement
  - `build` — build system, dependencies, or the Dockerfile
  - `ci` — CI configuration (`.github/workflows/`)
  - `chore` — anything else that doesn't touch `src/` behaviour
- **scope** *(optional)* — the area touched, e.g. `routing`, `sde`, `eve-scout`,
  `health`, `openapi`.
- **description** — imperative mood, lower-case, no trailing period
  ("add", not "added"/"adds").

### Breaking changes

Append `!` after the type/scope and add a `BREAKING CHANGE:` footer:

```
feat(routing)!: rename preference `less_safe` to `prefer_gates`

BREAKING CHANGE: the `less_safe` preference value is no longer accepted.
```

### Examples

```
feat: add gate-routing foundation
fix(routing): label a parallel-gate wormhole step as stargate
docs: document the PALU_SDE_DIR offline override
ci: publish the palu image to GHCR on develop and tags
test(eve-scout): cover expired-signature dropping
```

## Before pushing

Run the full gate (mirrors CI):

```sh
just check    # fmt + clippy + test + hurl
```

## Spec-driven workflow

Behavioural changes go through the OpenSpec workflow in `openspec/`
(proposals, specs, and tasks under `openspec/changes/`). See
[`AGENTS.md`](AGENTS.md).
