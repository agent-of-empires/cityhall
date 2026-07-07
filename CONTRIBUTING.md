# Contributing to CityHall

Thanks for helping out. This guide covers the basics.

## Workflow

1. Fork or branch off `main`.
2. Make your change with a focused commit history.
3. Open a PR. Fill out the PR template.
4. CI must pass and a code owner must approve before merge.

## Commit and PR titles

We squash-merge, so the PR title becomes the commit subject. It must follow
[Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add session export endpoint
fix: correct off-by-one in pagination
```

Accepted types: `feat`, `fix`, `perf`, `security`, `revert`, `chore`,
`build`, `ci`, `docs`, `style`, `refactor`, `test`. Subjects start lowercase.
A CI check (`pr-title-check`) enforces this.

## Style

- Rust: `cargo fmt` and `cargo clippy` must be clean.
- Web: `npm run format` and `npm run lint` must be clean.
- Do not use em dashes in prose.

Install the pre-commit hooks so these run before each commit:

```sh
pre-commit install
```

## Tests

Add or update tests alongside behavior changes. Rust: `cargo test`. Web: `npm test`.

## Code of Conduct

Participation is governed by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
