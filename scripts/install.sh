#!/usr/bin/env bash
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
  echo "Run with sudo: sudo scripts/install.sh" >&2
  exit 1
fi

project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
release_dir="$project_dir/target/release"

for binary in multidesktopd multi-desktop-session multi-desktop-capture-agent multi-desktop-media-agent multi-desktop-audio-agent multi-desktop-input-agent multi-desktop-session-inner multidesktopctl; do
  if [[ ! -x "$release_dir/$binary" ]]; then
    echo "Missing $release_dir/$binary. Run cargo build --release first." >&2
    exit 1
  fi
done

install -Dm755 "$release_dir/multidesktopd" /usr/local/bin/multidesktopd
install -Dm755 "$release_dir/multi-desktop-session" /usr/local/bin/multi-desktop-session
install -Dm755 "$release_dir/multi-desktop-capture-agent" /usr/local/bin/multi-desktop-capture-agent
install -Dm755 "$release_dir/multi-desktop-media-agent" /usr/local/bin/multi-desktop-media-agent
install -Dm755 "$release_dir/multi-desktop-audio-agent" /usr/local/bin/multi-desktop-audio-agent
install -Dm755 "$release_dir/multi-desktop-input-agent" /usr/local/bin/multi-desktop-input-agent
install -Dm755 "$release_dir/multi-desktop-session-inner" /usr/local/bin/multi-desktop-session-inner
install -Dm755 "$release_dir/multidesktopctl" /usr/local/bin/multidesktopctl
install -Dm644 "$project_dir/packaging/multidesktopd.service" /etc/systemd/system/multidesktopd.service
if [[ ! -e /etc/multi-desktop/multi-desktop.conf ]]; then
  install -Dm600 "$project_dir/config/multi-desktop.conf.example" /etc/multi-desktop/multi-desktop.conf
  echo "Created /etc/multi-desktop/multi-desktop.conf; set a unique token before enabling the service."
else
  echo "Kept existing /etc/multi-desktop/multi-desktop.conf."
fi

systemctl daemon-reload
echo "Installed Multi-Desktop. The service is not enabled or started."
