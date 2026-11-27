FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends openssh-server iproute2 \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --shell /bin/bash uksfta \
    && usermod -p '*' uksfta \
    && mkdir -p /home/uksfta/upload /home/uksfta/.ssh \
    && chown -R uksfta:uksfta /home/uksfta \
    && chmod 700 /home/uksfta/.ssh

COPY sshd_config /etc/ssh/sshd_config
COPY netem.sh /netem.sh
COPY shape.sh /shape.sh
COPY keys/id_ed25519_host /etc/ssh/id_ed25519_host
COPY keys/id_ed25519_host.pub /etc/ssh/id_ed25519_host.pub
COPY keys/authorized_keys /home/uksfta/.ssh/authorized_keys
RUN chown uksfta:uksfta /home/uksfta/.ssh/authorized_keys \
    && chmod 600 /home/uksfta/.ssh/authorized_keys \
    && chmod 600 /etc/ssh/id_ed25519_host \
    && mkdir -p /run/sshd

EXPOSE 2222
CMD ["/usr/sbin/sshd", "-D", "-e"]
