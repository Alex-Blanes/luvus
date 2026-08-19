#!/bin/sh
# Open or close a command group in the dock, then repaint it.
#
# The clicked group's id arrives as the row's value. Clicking the group that is
# already open closes it, so one row is both the opener and the closer — the
# same gesture the FILES tree uses on a folder.
#
# Only one group is open at a time (see dock.sh: module docks do not scroll), so
# this is a single-value file rather than a set.
set -eu
. "$(dirname "$0")/lib.sh"

want="${LUVUS_MODULE_ROW_VALUE:-}"
[ -n "$want" ] || exit 0

now=$(cat "$state/expanded" 2>/dev/null || printf '')
if [ "$now" = "$want" ]; then
  # Collapse: remove the file rather than writing an empty one, so a stale
  # zero-byte file can never read as "some group is open".
  rm -f "$state/expanded"
else
  mkdir -p "$state"
  printf '%s' "$want" > "$state/expanded"
fi

sh "$(dirname "$0")/dock.sh"
