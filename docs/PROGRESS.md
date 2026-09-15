# Real progress toward Multi-Desktop v1.0

Last reviewed: 2026-09-15.

This document intentionally separates implemented code from the actual user outcome. The project can display and hear a selected virtual desktop through local FFmpeg tools and send discrete reference keyboard/pointer events, but it lacks A/V synchronization and continuous client input capture. Therefore it is not usable as a full remote-desktop product yet.

## Update discipline

Every completed implementation change must update this document in the same handoff. Record only behavior backed by code or a documented test, adjust the relevant milestone and completion estimate, and leave unfinished user outcomes explicitly marked as pending.

## Honest status

| Area | Weight | Current completion | Evidence | What is missing |
| --- | ---: | ---: | --- | --- |
| Product boundaries and architecture | 10% | 80% | Architecture, isolation rules and a versioned frame format exist. | Threat model review and ADRs for codec/TLS choices. |
| Host session lifecycle | 20% | 45% | Per-desktop user, HOME, DBus, runtime and systemd transient unit are implemented. | End-to-end launch/restart/crash recovery tests on real hardware. |
| Pairing and authorization | 15% | 75% | One-use five-minute pairing codes, hashed token verifiers, device listing/targeted revocation, TLS server/client validation and a per-IP failed-auth limit are implemented. | Device keys/mTLS, secure client-side storage and token rotation. |
| Video pipeline | 20% | 35% | The private Gamescope PipeWire node is encoded to H.264, carried in bounded authenticated frames, saved by the CLI or displayed by a local FFmpeg reference viewer. | Congestion control and a packaged Windows decoder/renderer. |
| Audio pipeline | 10% | 40% | A private null sink is captured as Opus/Ogg, carried in bounded authenticated frames and saved or played by the CLI. | A/V synchronization, adaptive buffering and packaged Windows playback. |
| Remote input | 10% | 60% | Authenticated bounded keyboard/pointer/button/scroll events reach only the selected Gamescope-private libei agent; disconnect resets logical state. | Continuous client capture, controller support and hardware isolation tests. |
| Windows client | 10% | 20% | Portable Rust CLI builds, displays reference H.264 and sends discrete input events. | Pairing UX, secure credential storage, packaged video/audio renderer and continuous input capture. |
| Packaging, observability and QA | 5% | 40% | Install/uninstall scripts, `doctor`, audit events, unit tests and Linux/Windows-client CI exist. | Firewall integration, updates, logs UI, integration tests and signed release artifacts. |

**Weighted project completion: about 55% of v1.0.**

**End-to-end user outcome completion: reference audiovisual and input paths.** A laptop can use FFmpeg to see/hear the session and send discrete test input, but has no integrated client, synchronized playback or continuous controls.

## What already works in code

- A root daemon parses validated configuration and controls desktop lifecycle requests.
- A desktop can have a dedicated Linux user, persistent private home, runtime directory, DBus session and transient systemd unit.
- Each desktop launches the minimal PipeWire server needed for its Gamescope capture source inside the private runtime; physical audio routing is not enabled.
- A private Pulse-compatible server exposes only a virtual null sink; its monitor is encoded as 48 kHz stereo Opus/Ogg and relayed through an authenticated desktop-scoped connection.
- The CLI can save the Ogg stream or play it through locally installed `ffplay`; audio and video are not synchronized.
- The default backend is a headless Gamescope + XFCE session, not the local Hyprland session.
- The control protocol has bounded messages, timeouts, protocol identification, an information endpoint and audit events.
- A host can mint a high-entropy, single-use pairing code; redeeming it creates a desktop-scoped token.
- The project has a shared Rust library for future host/client protocol code and a `doctor` command for host prerequisites.
- A session-only capture agent finds the private Gamescope PipeWire node, and a media agent encodes H.264 directly into a Unix relay inside that desktop's private runtime.
- The daemon can authorize `VIDEO <desktop-id>`, derive the selected desktop's Unix relay path and forward it as versioned binary frames. The portable client saves an Annex-B `.h264` stream.
- The CLI also has `WATCH <desktop-id>`, which launches a locally installed FFmpeg `ffplay` process as a low-latency reference viewer; it is not the future packaged renderer.
- A non-loopback control listener requires TLS. The cross-platform CLI verifies a configured PEM trust anchor and server name.
- A safe helper generates non-overwriting self-signed TLS material with a DNS/IP subject alternative name for trusted-LAN setups.
- An administrator can revoke every paired-device token for one desktop without deleting its persistent files or stopping its session.
- The host persists SHA-256 token verifiers instead of bearer tokens, migrates legacy records on successful use, and supports administrator-only device listing and targeted revocation.
- Five failed authentication or pairing attempts per IP are limited for one minute, without logging the attempted secret.
- The default Gamescope command starts an inner private-session launcher and libei agent. A separately authenticated channel forwards validated evdev keyboard, pointer, button and scroll events only to the selected desktop's derived private socket.
- The input agent allows one active channel, tracks logical key/button state and releases it on reset, malformed input or disconnect.
- Video and input Unix relays reject symlinks/non-sockets and verify the connected process UID against the selected provisioned desktop before transferring data.
- Capture, media and input agents are restarted inside the selected session's cgroup after an unexpected exit.
- CI checks formatting, unit tests, clippy and release builds on Linux, plus the portable library and reference client on Windows.

## What does **not** work yet

- No packaged Windows decoder/renderer consumes the H.264 frames live. `WATCH` requires a local FFmpeg installation as a temporary reference viewer.
- No congestion control, live decode/render path, bitrate control or packet-loss recovery exists.
- No A/V synchronization, adaptive audio buffer or packaged Windows audio playback exists.
- No continuous Windows keyboard/mouse capture or controller path exists. The CLI can send discrete reference events only.
- No mutual TLS/device keys exist. Plaintext control is restricted to loopback development; LAN control uses server-authenticated TLS only.
- No GUI client, secure client-side credential store, token rotation or real installation wizard exists.
- No full isolation or performance test has been run with both the physical desktop and a remote session under load.

## Roadmap to the first genuinely useful build

### Milestone A — Verified isolated host sessions

Goal: reliably start and recover a second virtual desktop while the physical desktop remains unaffected.

- [x] Add structured session state and failure reason reporting (`DETAILS <desktop-id>`).
- [x] Add crash/restart policy and cleanup of stale runtime directories.
- [ ] Add integration tests that provision, start, stop and restart a disposable desktop.
- [ ] Verify that local Hyprland input, sockets and HOME are inaccessible from the remote session.
- [ ] Document GPU/render-node permissions for AMD, Intel and NVIDIA.

Acceptance: two sessions run on the same host and a failure in the virtual session cannot affect the physical desktop.

### Milestone B — Secure device access

Goal: a laptop receives only the rights for its own desktop.

- [~] Replace the development control channel with TLS and certificate/device pairing. Server-authenticated TLS is implemented; device certificates remain.
- [~] Protect device credentials at rest and add revoke/list/rotate commands. Non-reversible token verifiers, listing and both targeted/all-device revocation exist; client-side secure storage and rotation remain.
- [~] Add per-IP and per-code rate limits plus audit-safe logs. Per-IP limiting and secret-safe audit logs exist; per-code limits remain.
- [ ] Add a firewall-aware installer that leaves loopback as the safe default.

Acceptance: a paired token cannot access another desktop, replay a pairing code, or connect without encrypted authentication.

### Milestone C — First video path

Goal: Windows displays the correct virtual desktop at usable latency.

- [~] Capture only the selected virtual compositor, never the physical desktop. The session discovers its private Gamescope PipeWire node, encodes H.264 and carries it to a reference viewer; packaged client support is next.
- [~] Implement hardware-accelerated H.264/AV1 encode abstraction. A CPU H.264 baseline exists; hardware backends and an abstraction remain.
- [~] Implement encrypted video transport with loss recovery and bitrate adaptation. TLS-protected, bounded H.264 framing exists; loss recovery and adaptation remain.
- [ ] Implement a Windows decoder and renderer. The reference client can save raw H.264 or delegate live playback to locally installed FFmpeg.
- [ ] Measure latency, CPU/GPU use and behavior under packet loss.

Acceptance: the laptop displays the selected virtual desktop at 1080p60 on the LAN while the physical desktop remains private.

### Milestone D — Audio and input

Goal: the remote user can use the desktop normally without cross-input.

- [~] Per-session audio capture and playback with synchronization. Private virtual-sink Opus capture and reference playback exist; synchronization and packaged playback remain.
- [~] Virtual keyboard, pointer and controller devices scoped to one desktop. Keyboard/pointer/button/scroll forwarding and disconnect cleanup exist through private libei; continuous capture and controllers remain.
- [ ] Clipboard policy and explicit file-transfer policy.
- [~] Focus, reconnect and disconnect safety behavior. The agent releases held state and ends emulation on reset/disconnect; GUI focus and reconnection policy remain.

Acceptance: keyboard/mouse/controller from the laptop affect only its virtual desktop; physical input affects only the physical seat.

### Milestone E — Product release candidate

Goal: safe installable software that other people can use.

- [ ] Windows GUI with pairing, desktop selection, status and error reporting.
- [ ] Host setup wizard, dependency detection and safe updates.
- [~] CI for Linux host and Windows client builds; reproducible release artifacts. Build/test jobs exist for both platforms; reproducible signed artifacts remain.
- [ ] Soak tests with two active desktops, games and normal desktop workloads.
- [ ] Security review, privacy policy, license notices and user documentation.

Acceptance: a new user can install the host and Windows client, pair a laptop, use an isolated desktop for normal work and games, and recover from disconnects without touching the physical session.

## v1.0 definition of done

All of these must be true:

- [ ] The Windows client pairs over encrypted transport and stores credentials safely.
- [ ] A pairing automatically provisions the associated isolated desktop user.
- [ ] Video, audio and input work at a documented LAN performance target.
- [ ] Remote input is provably unable to reach the physical desktop.
- [ ] The physical desktop is never capturable by a remote desktop token.
- [ ] A device can be revoked without deleting the desktop's files.
- [ ] Host setup, firewall guidance, diagnostics, updates and removal are documented and tested.
- [ ] Automated unit, integration, security and end-to-end tests pass on supported Linux hosts and Windows clients.
