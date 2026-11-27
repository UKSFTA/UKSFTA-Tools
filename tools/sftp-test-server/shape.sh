#!/bin/sh
# Shape the fixture's own interface. Run this only inside the bridge
# container, never on a host interface.
set -eu

if [ ! -e /.dockerenv ]; then
    echo "shape.sh: refusing to run outside a container" >&2
    exit 1
fi

device="${2:-eth0}"

case "${1:-}" in
    apply)
        tc qdisc replace dev "$device" root netem delay 60ms 10ms rate 40mbit
        ;;
    clear)
        tc qdisc del dev "$device" root 2>/dev/null || true
        ;;
    loss)
        tc qdisc change dev "$device" root netem delay 60ms 10ms loss "${3:-100}%"
        ;;
    *)
        echo "usage: shape.sh apply|clear|loss [percent]" >&2
        exit 2
        ;;
esac
