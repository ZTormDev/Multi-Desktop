# Real progress toward Multi-Desktop v1.0

Last reviewed: 2026-09-15.

This document intentionally separates implemented code from the actual user outcome. The project has a credible host-control foundation, but **it cannot yet display, hear, or control a remote desktop from Windows**. Therefore it is not usable as a remote-desktop product yet.

## Honest status

| Area | Weight | Current completion | Evidence | What is missing |
| --- | ---: | ---: | --- | --- |
| Product boundaries and architecture | 10% | 80% | Architecture, isolation rules and a versioned frame format exist. | Threat model review and ADRs for codec/TLS choices. |
| Host session lifecycle | 20% | 45% | Per-desktop user, HOME, DBus, runtime and systemd transient unit are implemented. | End-to-end launch/restart/crash recovery tests on real hardware. |
| Pairing and authorization | 15% | 45% | One-use five-minute pairing codes and desktop-scoped device tokens are implemented and tested. | TLS, revocation, rate limiting, encrypted credential storage and multi-device management. |
| Video pipeline | 20% | 0% | Only future frame types are defined. | Capture, encode, congestion control, decode and rendering. |
| Audio pipeline | 10% | 0% | Only future frame types are defined. | Per-session capture, Opus/AAC decision, sync and playback. |
| Remote input | 10% | 0% | Isolation requirement is documented. | Authenticated input channel, virtual devices and focus rules. |
| Windows client | 10% | 5% | Portable Rust control CLI builds. | Pairing UX, secure credential storage, video/audio renderer and input capture. |
| Packaging, observability and QA | 5% | 30% | Install/uninstall scripts, `doctor`, audit events and unit tests exist. | Firewall integration, updates, logs UI, integration tests and release CI. |

**Weighted project completion: about 27% of v1.0.**

**End-to-end user outcome completion: 0%.** The key outcome starts only when the laptop can see and interact with its own isolated desktop without influencing the physical one.

## What already works in code

- A root daemon parses validated configuration and controls desktop lifecycle requests.
- A desktop can have a dedicated Linux user, persistent private home, runtime directory, DBus session and transient systemd unit.
- The default backend is a headless Gamescope + XFCE session, not the local Hyprland session.
- The control protocol has bounded messages, timeouts, protocol identification, an information endpoint and audit events.
- A host can mint a high-entropy, single-use pairing code; redeeming it creates a desktop-scoped token.
- The project has a shared Rust library for future host/client protocol code and a `doctor` command for host prerequisites.

## What does **not** work yet

- No screen capture exists, so a Windows laptop cannot see a desktop.
- No encoder, decoder, bitrate control or video transport exists.
- No audio capture or playback exists.
- No virtual keyboard/mouse path exists, so the laptop cannot control a remote desktop.
- No TLS exists. The current pairing/control channel is for trusted-LAN development only.
- No GUI client, persistent secure credential store, device revoke command or real installation wizard exists.
- No full isolation or performance test has been run with both the physical desktop and a remote session under load.

## Roadmap to the first genuinely useful build

### Milestone A — Verified isolated host sessions

Goal: reliably start and recover a second virtual desktop while the physical desktop remains unaffected.

- [x] Add structured session state and failure reason reporting (`DETAILS <desktop-id>`).
- [ ] Add crash/restart policy and cleanup of stale runtime directories.
- [ ] Add integration tests that provision, start, stop and restart a disposable desktop.
- [ ] Verify that local Hyprland input, sockets and HOME are inaccessible from the remote session.
- [ ] Document GPU/render-node permissions for AMD, Intel and NVIDIA.

Acceptance: two sessions run on the same host and a failure in the virtual session cannot affect the physical desktop.

### Milestone B — Secure device access

Goal: a laptop receives only the rights for its own desktop.

- [ ] Replace the development control channel with TLS and certificate/device pairing.
- [ ] Encrypt device credentials at rest and add revoke/list/rotate commands.
- [ ] Add per-IP and per-code rate limits plus audit-safe logs.
- [ ] Add a firewall-aware installer that leaves loopback as the safe default.

Acceptance: a paired token cannot access another desktop, replay a pairing code, or connect without encrypted authentication.

### Milestone C — First video path

Goal: Windows displays the correct virtual desktop at usable latency.

- [ ] Capture only the selected virtual compositor, never the physical desktop.
- [ ] Implement hardware-accelerated H.264/AV1 encode abstraction.
- [ ] Implement encrypted video transport with loss recovery and bitrate adaptation.
- [ ] Implement a Windows decoder and renderer.
- [ ] Measure latency, CPU/GPU use and behavior under packet loss.

Acceptance: the laptop displays the selected virtual desktop at 1080p60 on the LAN while the physical desktop remains private.

### Milestone D — Audio and input

Goal: the remote user can use the desktop normally without cross-input.

- [ ] Per-session audio capture and playback with synchronization.
- [ ] Virtual keyboard, pointer and controller devices scoped to one desktop.
- [ ] Clipboard policy and explicit file-transfer policy.
- [ ] Focus, reconnect and disconnect safety behavior.

Acceptance: keyboard/mouse/controller from the laptop affect only its virtual desktop; physical input affects only the physical seat.

### Milestone E — Product release candidate

Goal: safe installable software that other people can use.

- [ ] Windows GUI with pairing, desktop selection, status and error reporting.
- [ ] Host setup wizard, dependency detection and safe updates.
- [ ] CI for Linux host and Windows client builds; reproducible release artifacts.
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
