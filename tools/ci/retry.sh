#!/usr/bin/env bash
# retry.sh N COMMAND...: run COMMAND up to N times, 10 s, 20 s, ... apart, so that
# one failed download (a toolchain, a tool) does not turn a job red. Exits with
# the last attempt's status.
set -u
tries=$1
shift
status=0
for attempt in $(seq 1 "$tries"); do
  "$@" && exit 0
  status=$?
  if [ "$attempt" -lt "$tries" ]; then
    echo "::warning::attempt $attempt of $tries failed with status $status: $*"
    sleep $((10 * attempt))
  fi
done
exit "$status"
