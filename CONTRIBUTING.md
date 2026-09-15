# Contributing

Multi-Desktop is built around strict boundaries between desktop lifecycle,
capture, encoding, transport, audio, input and clients. Keep changes within
one boundary whenever possible.

Before opening a pull request, run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

Do not add a feature that can inject remote input into the physical desktop
without an explicit desktop-selection boundary and a test for it.
