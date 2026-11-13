# Definition of done

A task is done when all of these hold:

- Tests were written first and pass.
- `cargo fmt --check` is clean.
- `cargo clippy --all-targets -- -D warnings` is clean.
- No new visible string literal outside `src/i18n`.
- No SQL outside `src/infrastructure`.
- Every new write path to stored data goes through `AppAction` and `Store::apply`.
- `integrity::check` is clean at the end of every new repository test.
- A new screen has a UI test that drives it, and a snapshot if it is a top-level screen.
- One manual check is noted in the commit message.

Before a release, also run `docs/superpowers/doc-lint.sh` and the checklist in `docs/smoke-1.0.md`.
