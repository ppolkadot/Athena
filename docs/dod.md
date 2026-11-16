# Definition of done

A task is done when all of these hold:

- Tests were written first and pass.
- Task cites its `contracts.md` section.
- CI is green.
- `cargo fmt --check` is clean.
- `cargo clippy --all-targets --locked -- -D warnings` is clean.
- No new visible string literal outside `src/i18n`.
- No SQL outside `src/infrastructure`.
- Every new write path to stored data goes through `AppAction` and `Store::apply`.
- `integrity::check` is clean at the end of every new repository test.
- A new screen has a UI test that drives it, and a snapshot if it is a top-level screen.
- One manual check is noted in the commit message.
- Release checks are bound to the recorded final artifact SHA-256 hashes.

Before a release, also run `docs/superpowers/doc-lint.sh` and the release smoke checklist (`docs/smoke-1.0.md` or the release's `docs/smoke-1.x.md`).
