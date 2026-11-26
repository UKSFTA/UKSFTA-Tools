#!/bin/sh
# Shape the container link for the resume and throughput tests. Wave 1 does
# not use this; Wave 4 does.
set -eu
tc qdisc replace dev eth0 root netem delay 60ms 10ms rate 40mbit
