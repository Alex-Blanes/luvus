#!/bin/sh
# Open the serial monitor and remember which pane it lives in.
#
# The pane id is recorded so `flash-monitor.sh` can find the monitor later,
# stop it for the flash, and restart it in the *same* pane — which is what
# preserves the log from before the flash.
set -eu
. "$(dirname "$0")/lib.sh"
require_idf

pane=$(target_pane)
[ -n "$pane" ] || { toast "no pane to run in"; exit 1; }

printf '%s' "$pane" > "$state/monitor-pane"
"$luvus" pane run "$pane" "$(idf_cmd monitor)"
"$luvus" pane focus "$pane" >/dev/null 2>&1 || true
