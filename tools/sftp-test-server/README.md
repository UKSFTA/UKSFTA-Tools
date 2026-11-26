# SFTP test server

A throwaway OpenSSH SFTP server for the native transport and the segmented
upload tests. The keys in `keys/` are test-only. Never reuse them.

The server listens on container port 2222. The compose file publishes it on
host ports 2222 and 2223, so the non-default-port test can use the same
server.

Generate the throwaway keys first. The `keys/` directory is ignored by git,
so a fresh clone has none, and the image build needs them:

```bash
./tools/sftp-test-server/setup.sh
```

```bash
docker compose -f tools/sftp-test-server/docker-compose.yml up -d --build
docker compose -f tools/sftp-test-server/docker-compose.yml ps
docker compose -f tools/sftp-test-server/docker-compose.yml down -v
```

The service uses host networking and one `sshd` on ports 2222 and 2223, so
no port publishing is needed. On a host where the Docker build cannot reach
the registry, build first with the host network and start without a build:

```bash
docker build --network=host -t uksfta-sftp-test tools/sftp-test-server
docker compose -f tools/sftp-test-server/docker-compose.yml up -d --no-build
```

Connect by hand:

```bash
sftp -P 2222 -i tools/sftp-test-server/keys/id_ed25519 \
  -o StrictHostKeyChecking=no uksfta@127.0.0.1
```

Force-kill a transfer, to test resume:

```bash
docker compose -f tools/sftp-test-server/docker-compose.yml kill sftp
docker compose -f tools/sftp-test-server/docker-compose.yml up -d
```

Apply the shaped link for the netem tests:

```bash
docker compose -f tools/sftp-test-server/docker-compose.yml exec sftp \
  sh /netem.sh
```

## Live tests

Start the server, then:

```bash
UKSFTA_SFTP_LIVE=1 cargo test --locked -- --ignored native::tests
```

Without `UKSFTA_SFTP_LIVE=1` every live test is ignored.
