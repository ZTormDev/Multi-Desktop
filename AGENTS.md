# Working on Multi-Desktop

## Product boundary

Multi-Desktop turns one Linux host into multiple separately managed desktop sessions. The physical local user and every remote desktop are separate trust domains. A remote client must never receive the physical user's Wayland socket, input devices, HOME, runtime directory or DBus session.

This project owns the host, protocol and client. Do not substitute Moonlight, Sunshine, Wolf, Apollo or another remote-desktop product for its core functionality. Libraries for focused jobs such as TLS, codecs or a GUI toolkit are acceptable only when their security and license are reviewed and the integration preserves this project's protocol and isolation model.

## Non-negotiable security rules

- Treat every network packet and client-supplied identifier as hostile.
- Keep control, video, audio and input channels versioned and independently authenticated.
- Bind all remote input to a selected virtual desktop; never inject it into the physical seat.
- A paired-device token may manage only its own desktop. Administrative credentials may manage all desktops.
- Do not log tokens, pairing codes, passwords, video frames or private user content.
- Pairing codes must be high entropy, expire, and become unusable atomically after redemption.
- Default to loopback binding. A LAN bind requires an explicit firewall-aware configuration path.
- Do not claim GPU hardware isolation when a GPU is shared. Document the distinction from session isolation.

## Architecture and ownership

| Area | Main location | Notes |
| --- | --- | --- |
| Shared protocol primitives | `src/lib.rs`, `src/transport.rs` | Must compile for Windows. |
| Linux control plane | `src/main.rs`, `src/protocol.rs` | Root-only and narrow. |
| Session lifecycle | `src/session.rs`, `src/bin/desktop_session.rs` | Owns users, systemd units and environment isolation. |
| Device pairing | `src/pairing.rs` | Owns one-time code and device-token persistence. |
| Reference CLI | `src/bin/client.rs` | Keep cross-platform. |
| Packaging | `scripts/`, `packaging/`, `config/` | Installers must preserve user data by default. |

## Engineering workflow

1. Read `docs/ARCHITECTURE.md` and `docs/PROGRESS.md` before changing a boundary.
2. Make the smallest coherent change; avoid mixing protocol, encoder and UI work in one patch.
3. Add or update tests for parsing, authorization, framing and failure paths.
4. Run before handoff:

   ```bash
   cargo fmt --check
   cargo test
   cargo clippy --all-targets -- -D warnings
   cargo build --release
   ```

5. Use `sudo multidesktopd doctor <config>` before any real host-session test.

## Operational safety

- Never enable a network service, alter firewall rules, create/remove a desktop user, or stop a running desktop without explicit user authorization.
- `scripts/uninstall.sh` must preserve `/etc/multi-desktop` and `/var/lib/multi-desktop` unless the user explicitly asks for data removal.
- Keep the default compositor backend configurable. Do not hard-code a personal username, host name, GPU model or desktop name.

## Completion standard

Do not call the product ready until every acceptance criterion in the `v1.0` section of `docs/PROGRESS.md` is checked off by automated and documented end-to-end tests.
