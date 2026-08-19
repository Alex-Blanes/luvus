#!/bin/sh
# The reason this module exists.
#
# `idf.py monitor` holds the serial port, so flashing means: stop the monitor,
# flash, start it again — and every ESP-IDF user loses their scrollback doing
# it, dozens of times a day. Because luvus owns real panes, the module can send
# the monitor a Ctrl+C, flash, and bring the monitor back **in the same pane**,
# so what the board printed before the flash stays on screen above what it
# prints after. That side-by-side is the actual work of embedded debugging.
set -eu
. "$(dirname "$0")/lib.sh"
require_idf

mon=""
[ -f "$state/monitor-pane" ] && mon=$(cat "$state/monitor-pane" 2>/dev/null || true)

# 1. Release the port. 0x03 is Ctrl+C; idf.py monitor exits on it. If the pane
#    is gone (user closed it) this is a harmless no-op and we just flash.
if [ -n "$mon" ] && "$luvus" pane status "$mon" >/dev/null 2>&1; then
  "$luvus" pane send "$mon" "$(printf '\003')" >/dev/null 2>&1 || true
  # idf.py needs a moment to close the port; flashing into a held port fails
  # with a confusing "could not open" rather than anything actionable.
  sleep 1
else
  mon=""
fi

# 2. Flash, in the monitor's own pane when there is one, so the flash log and
#    the boot log end up in one scrollback.
case "$flash_method" in
  dfu)  sub="dfu-flash" ;;
  jtag) sub="flash -c jtag" ;;
  *)    sub="flash" ;;
esac
pane="${mon:-$(target_pane)}"
[ -n "$pane" ] || { toast "no pane to run in"; exit 1; }

# 3. Put the monitor back — chained with `&&` into the SAME typed line, so it
#    only runs if the flash succeeded. Two separate `pane run` calls would not
#    do this: each one just types a line, so the monitor would start even after
#    a failed flash and scroll the error away under a fresh boot log.
cmd=$(idf_cmd "$sub")
if [ "$auto_monitor" = "true" ]; then
  printf '%s' "$pane" > "$state/monitor-pane"
  cmd="$cmd && $(idf_cmd monitor)"
fi
"$luvus" pane run "$pane" "$cmd"
"$luvus" pane focus "$pane" >/dev/null 2>&1 || true
