#!/usr/bin/env bash
# Generate the throwaway keys the test container needs. The keys/ directory
# is ignored by git, so this must run after a fresh clone. Safe to run again.
set -euo pipefail

cd "$(dirname "$0")"
mkdir -p keys

if [ ! -f keys/id_ed25519_host ]; then
	ssh-keygen -q -t ed25519 -N '' -C 'uksfta sftp test host key' -f keys/id_ed25519_host
fi
if [ ! -f keys/id_ed25519 ]; then
	ssh-keygen -q -t ed25519 -N '' -C 'uksfta sftp test client key' -f keys/id_ed25519
fi

cp keys/id_ed25519.pub keys/authorized_keys
chmod 600 keys/id_ed25519 keys/id_ed25519_host
chmod 644 keys/id_ed25519.pub keys/id_ed25519_host.pub keys/authorized_keys

echo "test keys ready in tools/sftp-test-server/keys"
