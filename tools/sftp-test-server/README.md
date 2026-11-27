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

## Shaped fixture

The shaped tests use a second compose file on a private bridge network. It
publishes no ports and shapes only the container's own `eth0`, so the host
network is never touched. `shape.sh` refuses to run outside a container, and
`netem.sh` is disabled.

```bash
docker build --network=host -t uksfta-sftp-test tools/sftp-test-server
docker compose -f tools/sftp-test-server/docker-compose.shaped.yml up -d --no-build
docker compose -f tools/sftp-test-server/docker-compose.shaped.yml exec -T shaped \
  sh /shape.sh apply
docker compose -f tools/sftp-test-server/docker-compose.shaped.yml exec -T shaped \
  sh /shape.sh clear
docker compose -f tools/sftp-test-server/docker-compose.shaped.yml down -v
```

The shape is `tc qdisc replace dev eth0 root netem delay 60ms 10ms rate
40mbit`. The tests find the container address with `docker inspect`, connect
to `<address>:2222`, and write a temporary `known_hosts` entry.

`sch_netem` must exist in the running kernel. On a kernel without it, `tc`
answers `Specified qdisc kind is unknown`, and the shaped tests skip with that
reason instead of failing. They report:

```text
skipped: the shaped bridge fixture is unusable: cannot shape the container
link: Error: Specified qdisc kind is unknown.
```

Do not load modules or run `tc` on the host to work around this. Run the
shaped suites on a host whose kernel provides `sch_netem`.

## Live tests

Start the server, then:

```bash
UKSFTA_SFTP_LIVE=1 cargo test --locked -- --ignored native::tests
UKSFTA_SFTP_LIVE=1 cargo test --locked --test sftp_live -- --ignored
```

Without `UKSFTA_SFTP_LIVE=1` every live test is ignored.

The shaped cases need the bridge fixture and shaping:

```bash
UKSFTA_SFTP_LIVE=1 UKSFTA_SFTP_SHAPED=1 \
  cargo test --locked --test throughput_live -- --ignored --nocapture
UKSFTA_SFTP_LIVE=1 UKSFTA_SFTP_SHAPED=1 \
  cargo test --locked --test sftp_live -- --ignored
```

Run the two shaped commands one after the other. They share one container and
one shaped link.
