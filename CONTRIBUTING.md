# Contributing

Multi-Desktop is built around strict boundaries between desktop lifecycle,
capture, encoding, transport, audio, input and clients. Keep changes within
one boundary whenever possible.

The reference CLI can receive authenticated H.264 (`VIDEO`), show it through
a locally installed FFmpeg viewer (`WATCH`), and send discrete authenticated
input events. These are development/reference paths, not replacements for the
future packaged Windows renderer and continuous input capture.

Before opening a pull request, run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

Do not add a feature that can inject remote input into the physical desktop
without an explicit desktop-selection boundary and a test for it.

Input work belongs inside `multi-desktop-session-inner`, after Gamescope has
created its private `LIBEI_SOCKET`. Never use global uinput injection as a
shortcut.
