#!/bin/bash
# run.sh AREA LABEL [ENV...]: one timed export into soft-paint's user folder.
S=/srv/burst-out/soft-paint
export NWN_ROOT=/srv/layonara-forge/game NWN_HOME=$S/home XDG_DATA_HOME=$S/xdg HOME=$S LS_TIMING=1
unset LANDSCAPER_MODULE_REPO LANDSCAPER_HAKS
A=$1 L=$2; shift 2
cd $S/areas && env "$@" systemd-run --user --scope -q -p MemoryMax=24G nice -n 5 \
  /usr/bin/time -f "$L wall=%e s user=%U s sys=%S s maxrss=%M KB exit=%x" \
  $S/engine/target/release/landscaper export --jobs 16 "$A" > "$S/logs/$L-export.log" 2>&1
tail -1 "$S/logs/$L-export.log"
