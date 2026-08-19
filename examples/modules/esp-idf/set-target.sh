#!/bin/sh
# Apply the chip chosen in Settings. `set-target` wipes the build directory and
# regenerates sdkconfig, so it is a deliberate action rather than something the
# module does behind your back when the setting changes.
set -eu
. "$(dirname "$0")/lib.sh"
require_idf

pane=$(target_pane)
[ -n "$pane" ] || { toast "no pane to run in"; exit 1; }
toast "set-target $target (this clears the build dir)"
"$luvus" pane run "$pane" "$(idf_cmd "set-target $target")"
"$luvus" pane focus "$pane" >/dev/null 2>&1 || true
