# Architecture

`Multi-Desktop` is split by trust boundary, not by a collection of scripts.

```text
Windows client
  └─ independently authenticated TLS control, video and input connections
       └─ multidesktopd (root control plane)
            └─ desktop session supervisor (one per provisioned user)
                 └─ Gamescope + apps + private DBus, PipeWire, runtime and HOME
                      ├─ PipeWire capture, H.264 and virtual-sink Opus agents
                      └─ private Unix input relay → libei sender
```

## Components

| Component | Responsibility | Privilege |
| --- | --- | --- |
| `multidesktopd` | Authenticates clients and controls desktop lifecycles. | root |
| `multi-desktop-session` | Creates one user's process environment and starts its compositor. | desktop user |
| `multi-desktop-session-inner` | Runs inside Gamescope and starts the libei input agent. | desktop user |
| `multi-desktop-input-agent` | Accepts one session-local event stream and injects it only through the Gamescope-private EIS socket. | desktop user |
| `multi-desktop-audio-agent` | Creates a private null sink and relays only its monitor as Opus/Ogg. | desktop user |
| `multidesktopctl` | Cross-platform control reference client; builds on Windows. | unprivileged |
| `transport` | Versioned, bounded binary envelope for H.264 video and future audio/input channels. | library |

## Isolation policy

Every provisioned desktop gets an independent Linux user, runtime directory, HOME, DBus session, private PipeWire/Pulse-compatible graph, process tree and systemd unit. New desktop users receive the `render` group but not the `audio` group. They do **not** receive the physical user's Wayland, PipeWire, PulseAudio or XDG runtime sockets. GPU render-node access will be explicit and scoped to the compositor backend, rather than granting a remote desktop the physical host seat. Session audio terminates in a virtual sink; physical audio routing remains disabled.

The audio path starts a Pulse-compatible server against the private PipeWire graph without WirePlumber. The audio agent loads only `module-null-sink`, captures `multi_desktop_audio.monitor`, and serves Opus/Ogg through a derived Unix socket whose peer UID is checked by the daemon. `AUDIO <desktop-id>` uses a separately authenticated network connection. A/V synchronization remains pending.

This is session isolation. A single physical GPU still shares rendering capacity and VRAM; hardware-level GPU isolation needs vGPU support or another GPU.

## Protocol evolution

`transport.rs` defines fixed-size headers with magic, version, type and bounded payload length. It deliberately distinguishes control, video, audio and input frames so those streams can scale independently and be moved to QUIC/TLS without changing client semantics.

The control protocol has explicit version negotiation (`INFO`), bounded input, timeouts and a fixed per-IP failed-authentication limit. A loopback listener may use plaintext for local development; a non-loopback listener is rejected unless a PEM certificate and private key are configured. The reference client pins the supplied CA/certificate and validates the server name. Before a Windows GUI is shipped, this must grow into mutual TLS or device keys and a firewall installer.

Device tokens contain 256 bits of randomness and are returned only once. Persistence uses namespaced SHA-256 verifiers, so a copied host credential directory does not directly reveal usable bearer tokens. An administrator can list non-secret verifier prefixes per desktop and revoke one device or every device assigned to that desktop. Legacy plaintext-filename records migrate on their next successful authentication.

After authenticating, `VIDEO <desktop-id>` is authorized against the same desktop boundary as lifecycle requests. The root daemon derives and validates a Unix-socket path inside that desktop's private runtime; no client or status file chooses the endpoint. After connecting it checks Linux `SO_PEERCRED` against the provisioned desktop UID before reading any data, closing the symlink/replacement race available to a compromised session. The media agent writes H.264 directly to that socket, avoiding TCP port collisions between desktops. The daemon emits bounded `VideoConfig` and `VideoData` frames over the authenticated connection. The portable client can write those frames to an Annex-B `.h264` file or use a locally installed `ffplay` as a low-latency reference viewer. It does not yet ship its own decoder or renderer.

Input follows a separate connection and repeats authentication before `INPUT-STREAM <desktop-id>` is authorized. Frames accept only bounded, validated keyboard, relative-pointer, button, scroll and reset events. The daemon derives a Unix-socket path under the selected desktop's private runtime, verifies its peer UID with `SO_PEERCRED`, and never accepts a path from a client. The agent listens there as the desktop user and sends events only through the `LIBEI_SOCKET` inherited inside Gamescope. One client may emulate at a time, duplicate logical transitions are filtered, and disconnect/reset releases held keys and buttons before ending emulation. No component accesses `/dev/uinput` or the physical seat.
