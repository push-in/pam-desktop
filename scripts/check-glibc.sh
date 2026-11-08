#!/bin/sh
# Fails when the binary $1 needs a glibc newer than $2 (for example 2.35).
set -eu
binary=$1
oldest=$2
needed=$(objdump -T "${binary}" | grep -o 'GLIBC_[0-9.]*' | sed 's/^GLIBC_//' | sort -uV | tail -n 1)
echo "${binary} needs glibc ${needed:-(none)}"
newest=$(printf '%s\n%s\n' "${needed:-0}" "${oldest}" | sort -V | tail -n 1)
[ "${newest}" = "${oldest}" ] || { echo "glibc ${oldest} is the oldest supported" >&2; exit 1; }
