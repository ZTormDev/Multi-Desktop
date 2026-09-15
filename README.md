# Multi-Desktop

Multi-Desktop is an experimental, self-hosted Linux control plane for turning one powerful computer into several isolated desktop sessions. The physical local desktop remains separate while each remote desktop has its own Linux user, HOME, runtime directory, DBus session and compositor process.

## Current scope

Version 0.1 manages desktop lifecycles only:

- authenticated LAN control protocol;
- automatic desktop-user provisioning;
- one systemd transient unit per desktop;
- process-level isolation from the physical user's HOME and runtime;
- a portable control client that builds on Windows, Linux and macOS.

It does not yet include video, audio or remote-input transport. Those parts must be encrypted and bound to the selected virtual desktop before this project is suitable for daily remote use.

## Important limitation

Session isolation is not hardware GPU partitioning. With one physical GPU, desktops share rendering capacity and VRAM. Strong GPU-level isolation requires supported vGPU/SR-IOV hardware or another GPU.

## Build and install

```bash
cargo build --release
sudo scripts/install.sh
```

The installer copies program files and creates a local-only configuration at `/etc/multi-desktop/multi-desktop.conf` if one does not exist. It never enables the network service automatically.

Set a random token in that file, validate it, then explicitly enable the service:

```bash
sudo multidesktopd check /etc/multi-desktop/multi-desktop.conf
sudo systemctl enable --now multidesktopd.service
```

To permit a remote device, bind `listen` to the host's fixed LAN IP and add a firewall rule restricted to trusted devices. Do not expose the current development protocol to the Internet.

## Control protocol

After `AUTH <token>`, the client may issue:

```text
PING
INFO
LIST
PROVISION <desktop-id>
STATUS <desktop-id>
START <desktop-id>
STOP <desktop-id>
```

The development CLI is:

```bash
printf '%s' '<token>' | multidesktopctl <host:port> --token-stdin START <desktop-id>
```

Passing the token directly as an argument remains supported for simple tests, but standard input avoids exposing it in the local process list.

Lifecycle operations and failed authentication attempts are recorded in the daemon journal. Inspect them with `journalctl -u multidesktopd.service` after the service is enabled.

See [the architecture document](docs/ARCHITECTURE.md) for the boundary model. `scripts/uninstall.sh` removes installed program files but deliberately preserves configuration and all provisioned desktop data.
