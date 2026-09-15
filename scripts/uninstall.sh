#!/usr/bin/env bash
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
  echo "Run with sudo: sudo scripts/uninstall.sh" >&2
  exit 1
fi

systemctl disable --now multidesktopd.service 2>/dev/null || true
rm -f /usr/local/bin/multidesktopd /usr/local/bin/multi-desktop-session /usr/local/bin/multi-desktop-capture-agent /usr/local/bin/multidesktopctl
rm -f /etc/systemd/system/multidesktopd.service
systemctl daemon-reload
echo "Removed program files. Preserved /etc/multi-desktop and /var/lib/multi-desktop."
