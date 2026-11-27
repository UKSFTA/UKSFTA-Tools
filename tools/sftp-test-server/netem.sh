#!/bin/sh
# Superseded by shape.sh, which refuses to run on a host interface. This
# file remains so an old image keeps a harmless script.
echo "netem.sh: use 'docker compose -f docker-compose.shaped.yml exec -T shaped sh /shape.sh apply'" >&2
exit 1
