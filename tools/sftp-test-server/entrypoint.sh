#!/bin/sh
# Install the repository's throwaway keys on every container start.
#
# The image bakes a copy of the keys, so a key pair regenerated after the image
# build makes that copy stale and sshd rejects the client. A bind mount of
# keys/authorized_keys cannot carry the mode sshd expects either, because the
# host file mode wins. Copying from the read-only mount fixes both: the content
# always matches keys/, and the mode and owner are set here.
set -eu

install -m 600 -o uksfta -g uksfta /fixture-keys/authorized_keys \
	/home/uksfta/.ssh/authorized_keys
install -m 600 -o root -g root /fixture-keys/id_ed25519_host \
	/etc/ssh/id_ed25519_host
install -m 644 -o root -g root /fixture-keys/id_ed25519_host.pub \
	/etc/ssh/id_ed25519_host.pub

exec "$@"
