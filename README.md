# Multi-Desktop

Multi-Desktop is an experimental, self-hosted Linux control plane for turning one powerful computer into several isolated desktop sessions. The physical local desktop remains separate while each remote desktop has its own Linux user, HOME, runtime directory, DBus session and compositor process.

## Current scope

Version 0.1 has a host-side foundation for isolated sessions:

- authenticated control protocol, with mandatory TLS for LAN binding;
- automatic desktop-user provisioning;
- one systemd transient unit per desktop;
- process-level isolation from the physical user's HOME and runtime;
- a private PipeWire graph for each desktop's Gamescope capture source;
- private Gamescope PipeWire capture discovery, H.264 encoding and authenticated frame relay;
- a virtual per-session audio sink, Opus/Ogg relay and FFmpeg reference playback;
- a portable control client that builds on Windows, Linux and macOS, can save the stream or show it with locally installed FFmpeg;
- authenticated keyboard and pointer events routed through a private Gamescope/libei input agent.

Video, audio and discrete input events now have authenticated, versioned reference paths. There is still no packaged Windows renderer/player, synchronized A/V or continuous desktop input capture, so it is not suitable for daily remote use.

## Important limitation

Session isolation is not hardware GPU partitioning. With one physical GPU, desktops share rendering capacity and VRAM. Strong GPU-level isolation requires supported vGPU/SR-IOV hardware or another GPU.

## Build and install

```bash
cargo build --release
sudo scripts/install.sh
```

The installer copies program files and creates a local-only configuration at `/etc/multi-desktop/multi-desktop.conf` if one does not exist. It never enables the network service automatically.

Newly provisioned desktop users receive only the `render` supplementary group. If upgrading a development installation that previously created users in the `audio` group, review and remove that legacy membership before treating audio devices as isolated; Multi-Desktop does not automate account-policy changes on existing users.

Set a random token in that file, validate it, then explicitly enable the service:

```bash
sudo multidesktopd check /etc/multi-desktop/multi-desktop.conf
sudo multidesktopd doctor /etc/multi-desktop/multi-desktop.conf
sudo systemctl enable --now multidesktopd.service
```

For a trusted-LAN self-signed setup, generate certificate material for the host's fixed LAN IP (or DNS name) before enabling it:

```bash
sudo scripts/generate-tls-certificate.sh 192.168.1.10 /etc/multi-desktop/tls
```

Add the resulting paths to `multi-desktop.conf`, then copy only `server-cert.pem` to each client. The client must use the same IP/name that appears in the certificate's SAN.

`doctor` is read-only: it verifies the session launcher, systemd, DBus, private PipeWire service, user provisioning, H.264 GStreamer elements and the programs referenced by the default desktop command. Resolve every `FAIL` before enabling the service.

To permit a remote device, bind `listen` to the host's fixed LAN IP, configure `tls_certificate` and `tls_private_key`, and add a firewall rule restricted to trusted devices. The daemon rejects a non-loopback address without TLS. Do not expose the service to the Internet.

## Control protocol

After `AUTH <token>`, the client may issue:

```text
PING
INFO
LIST
PROVISION <desktop-id>
DEVICES <desktop-id>
REVOKE <desktop-id>
REVOKE-DEVICE <desktop-id> <device-id>
STATUS <desktop-id>
START <desktop-id>
STOP <desktop-id>
DETAILS <desktop-id>
RESTART <desktop-id>
CAPTURE <desktop-id>
MEDIA <desktop-id>
AUDIO-STATUS <desktop-id>
AUDIO <desktop-id>
INPUT <desktop-id>
INPUT-STREAM <desktop-id>
VIDEO <desktop-id>
```

The development CLI is:

```bash
printf '%s' '<token>' | multidesktopctl <host:port> --token-stdin START <desktop-id>
```

Passing the token directly as an argument remains supported for simple tests, but standard input avoids exposing it in the local process list.

Lifecycle operations and failed authentication attempts are recorded in the daemon journal. Inspect them with `journalctl -u multidesktopd.service` after the service is enabled.

Five failed `AUTH` or `PAIR` attempts from one IP address cause a 60-second temporary block. Successful authentication clears that origin's recent failures. Tokens and pairing codes are never written to the audit log.

`DETAILS <desktop-id>` returns the session's active state, sub-state, result and main-process exit status in one line. `STOP` removes only that desktop's temporary runtime directory; its provisioned HOME is kept.

Device bearer tokens are returned only at pairing time. The host stores a SHA-256 verifier rather than the token itself; records created by older builds are migrated after their next successful authentication. `DEVICES <desktop-id>` returns non-secret device IDs for administrator use. `REVOKE-DEVICE <desktop-id> <device-id>` invalidates one credential, while `REVOKE <desktop-id>` invalidates all credentials for that desktop. These operations preserve the account, persistent files and running session.

Virtual desktop units retry a crash up to three times per minute. `RESTART <desktop-id>` performs a controlled stop, cleans the temporary runtime and starts a fresh session.

`CAPTURE <desktop-id>` reports whether the session's private Gamescope PipeWire video node is ready. This is the capture source for the upcoming encoder; it does not capture the physical desktop.

`MEDIA <desktop-id>` reports the isolated session's local H.264 encoder state. Its Unix endpoint lives inside that desktop's private runtime and is never disclosed to a remote device.

`AUDIO-STATUS <desktop-id>` reports the session's virtual audio sink and Opus encoder state. The audio agent creates only a named null sink in the private Pulse-compatible server, then captures its monitor as 48 kHz stereo Opus in an Ogg stream. WirePlumber is deliberately not started, and newly provisioned users are not members of `audio`, so this path does not enumerate or open physical audio devices. PipeWire documents `module-null-sink` as a built-in Pulse-compatible module ([PipeWire null-sink documentation](https://docs.pipewire.org/page_pulse_module_null_sink.html)).

Save or listen to the authenticated reference stream without overwriting an existing output:

```bash
multidesktopctl --tls-ca server-cert.pem --server-name 192.168.1.10 \
  192.168.1.10:47990 --token-stdin AUDIO girlfriend session-audio.ogg
multidesktopctl --tls-ca server-cert.pem --server-name 192.168.1.10 \
  192.168.1.10:47990 --token-stdin LISTEN girlfriend
```

`LISTEN` uses a locally installed `ffplay`. Audio and video currently use separate authenticated connections and are not synchronized yet.

`INPUT <desktop-id>` reports the private input agent's capabilities and whether an input channel is active. `INPUT-STREAM <desktop-id>` upgrades a separately authenticated connection into bounded binary input frames. The daemon derives the selected session's Unix-socket path itself; the agent accepts only one active client and releases every held key/button when the channel resets or disconnects.

The portable CLI exposes small reference commands using evdev key codes and signed pointer/scroll deltas:

```bash
# Press and release the A key (evdev code 30), move, click and scroll.
multidesktopctl <host:port> --token-stdin INPUT-KEY girlfriend 30
multidesktopctl <host:port> --token-stdin INPUT-MOVE girlfriend 40 -10
multidesktopctl <host:port> --token-stdin INPUT-CLICK girlfriend left
multidesktopctl <host:port> --token-stdin INPUT-SCROLL girlfriend 0 -120
```

Use the TLS flags shown below for LAN connections. These commands prove the scoped event path but do not replace continuous keyboard/mouse capture in the future Windows UI.

`VIDEO <desktop-id>` upgrades the authenticated control connection into a versioned H.264 frame stream. On a LAN-enabled host it is protected by the same TLS connection and the daemon, not the laptop, connects to a derived Unix socket inside the selected session's private runtime. The reference client can save it without overwriting an existing file:

```bash
multidesktopctl --tls-ca server-cert.pem --server-name 192.168.1.10 \
  192.168.1.10:47990 --token-stdin VIDEO girlfriend recording.h264
```

The file is raw Annex-B H.264 for now; the Windows decoder/renderer is the next missing piece. The command ends when the remote session or media relay ends.

For an immediate live reference viewer, install FFmpeg on the laptop so `ffplay` is available, then use:

```bash
multidesktopctl --tls-ca server-cert.pem --server-name 192.168.1.10 \
  192.168.1.10:47990 --token-stdin WATCH girlfriend
```

`WATCH` starts `ffplay` locally with low-latency H.264 options and pipes it only frames from the authenticated desktop. It is a practical bridge while the project develops its own packaged Windows renderer; FFmpeg is not part of the Multi-Desktop host or protocol.

The default Gamescope command now starts a private inner launcher. It runs the input agent only inside the virtual desktop's child environment, where it connects to Gamescope's `LIBEI_SOCKET`; it never grants the remote desktop access to the physical host seat.

For a TLS-enabled host, pass the certificate authority (or self-signed server certificate) that you explicitly trust. The name must match a DNS/IP subject alternative name in that certificate:

```bash
multidesktopctl --tls-ca server-cert.pem --server-name 192.168.1.10 \
  192.168.1.10:47990 --token-stdin INFO
```

## Pair a remote device

On the host, create a one-use code for the desktop that belongs to the laptop owner:

```bash
sudo multidesktopd pair girlfriend
```

The code expires after five minutes. On the laptop, redeem it once:

```bash
multidesktopctl pair <host:port> <pairing-code>
```

That provisions the isolated Linux user and returns a device token. Save the token in the future client; it can only start, stop or view the paired desktop, never list or control anyone else's desktop. For a TLS-enabled host, place `--tls-ca` and `--server-name` before `pair` as shown above.

See [the architecture document](docs/ARCHITECTURE.md), [the real progress plan](docs/PROGRESS.md) and [the contributor/agent guide](AGENTS.md). `scripts/uninstall.sh` removes installed program files but deliberately preserves configuration and all provisioned desktop data.
