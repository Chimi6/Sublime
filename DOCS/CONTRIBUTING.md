# Contributing

## Code style

- One action per line. Prefer a named intermediate variable over a nested call.
- Descriptive names. No single-letter names outside tiny closures.
- No `unsafe`. No `unwrap()` or `expect()` outside tests.
- The library never prints. Emit events through the `Context` you are given.
- `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`
  must pass before every commit.

## Commits

Conventional commits: `feat:`, `fix:`, `docs:`, `ci:`, `test:`, `chore:`.
No trailers. Update `DOCS/CHANGELOG.md` and, when relevant, `DOCS/STATE.md`
in the same commit.
