#!/bin/sh
# Run one idf.py subcommand in a pane: `sh idf.sh build`, `sh idf.sh menuconfig`.
#
# It is typed into a real pane rather than run headless on purpose. menuconfig
# is a terminal UI, monitor is a serial console, and a build you cannot Ctrl+C
# is worse than useless — a multiplexer should just let them be what they are.
set -eu
. "$(dirname "$0")/lib.sh"
require_idf

# Invoked two ways: from a context-menu action (`sh idf.sh build`) or from a
# COMMANDS dock row, where the subcommand rides in as the row's `value`.
sub="${1:-}"
[ -n "$sub" ] || sub="${LUVUS_MODULE_ROW_VALUE:-build}"
pane=$(target_pane)
[ -n "$pane" ] || { toast "no pane to run in"; exit 1; }

"$luvus" pane run "$pane" "$(idf_cmd "$sub")"
"$luvus" pane focus "$pane" >/dev/null 2>&1 || true
