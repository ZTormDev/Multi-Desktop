#!/usr/bin/env bash
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
  echo "Run with sudo." >&2
  exit 1
fi

install -Dm755 target/release/multi-desktop-session /usr/local/bin/multi-desktop-session
echo "Generic session launcher installed. Desktop users are created on provisioning."
