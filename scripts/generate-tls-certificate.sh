#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "Usage: $0 <server-name-or-ip> <empty-output-directory>" >&2
  exit 2
fi

server_name=$1
output_dir=$2
certificate="$output_dir/server-cert.pem"
private_key="$output_dir/server-key.pem"

if ! command -v openssl >/dev/null 2>&1; then
  echo "openssl is required to generate TLS material." >&2
  exit 1
fi
if [[ -e $certificate || -e $private_key ]]; then
  echo "Refusing to overwrite existing TLS material in $output_dir." >&2
  exit 1
fi

umask 077
mkdir -p -- "$output_dir"

if [[ $server_name =~ ^[0-9]{1,3}(\.[0-9]{1,3}){3}$ || $server_name == *:* ]]; then
  subject_alt_name="IP:$server_name"
else
  subject_alt_name="DNS:$server_name"
fi

openssl req -x509 -newkey rsa:3072 -nodes \
  -keyout "$private_key" \
  -out "$certificate" \
  -days 365 \
  -subj "/CN=$server_name" \
  -addext "subjectAltName=$subject_alt_name"

chmod 600 "$private_key"
chmod 644 "$certificate"
echo "Created $certificate and $private_key"
echo "Trust server-cert.pem on each paired client and configure both paths in multi-desktop.conf."
