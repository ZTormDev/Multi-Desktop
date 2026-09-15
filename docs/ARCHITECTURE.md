# Architecture

`Multi-Desktop` is split by trust boundary, not by a collection of scripts.

```text
Windows client
  └─ encrypted transport (planned)
       └─ multidesktopd (root control plane)
            ├─ physical-host connector (read-only until explicitly selected)
            └─ desktop session supervisor (one per provisioned user)
                 └─ compositor + apps + private DBus + private HOME
```

## Components

| Component | Responsibility | Privilege |
| --- | --- | --- |
| `multidesktopd` | Authenticates clients and controls desktop lifecycles. | root |
| `multi-desktop-session` | Creates one user's process environment and starts its compositor. | desktop user |
| `multidesktopctl` | Cross-platform control reference client; builds on Windows. | unprivileged |
| `transport` | Versioned, bounded binary envelope for future media/input channels. | library |

## Isolation policy

Every provisioned desktop gets an independent Linux user, runtime directory, HOME, DBus session, process tree and systemd unit. It does **not** receive the physical user's Wayland socket or XDG paths. GPU render-node access will be explicit and scoped to the compositor backend, rather than granting a remote desktop the physical host seat.

This is session isolation. A single physical GPU still shares rendering capacity and VRAM; hardware-level GPU isolation needs vGPU support or another GPU.

## Protocol evolution

`transport.rs` defines fixed-size headers with magic, version, type and bounded payload length. It deliberately distinguishes control, video, audio and input frames so those streams can scale independently and be moved to QUIC/TLS without changing client semantics.

The current text control connection is development-only. It already has explicit version negotiation (`INFO`), bounded input and timeouts; before a Windows GUI is shipped, it will be replaced with mutual TLS, per-device pairing keys, rate limiting and a firewall installer.
