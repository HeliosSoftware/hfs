#!/bin/bash
# stop ONLY the real pg-es hfs (the one listening on 18080)
P=$(ss -ltnpH 'sport = :18080' | grep -o 'pid=[0-9]*' | head -1 | cut -d= -f2); [ -n "$P" ] && kill $P
for i in $(seq 1 60); do ss -ltn 'sport = :18080' | grep -q LISTEN || exit 0; sleep 1; done; exit 1
