#!/bin/sh
# Fake idf.py for testing the module without hardware.
#   - logs every call to $IDF_FAKE_LOG (default /tmp/idf-calls.log)
#   - `monitor` blocks and streams, like the real one holding the port
#   - create $IDF_FAKE_FAIL to make `flash` exit non-zero. The path is inherited
#     by the isolated test server and therefore reaches luvus-spawned panes.
log="${IDF_FAKE_LOG:-/tmp/idf-calls.log}"
fail="${IDF_FAKE_FAIL:-/tmp/idf-fake-fail}"
for a in "$@"; do sub="$a"; done          # subcommand is the LAST arg, after -p/-b
echo "IDFCALL $*" >> "$log"
case "$sub" in
  monitor)
    echo "--- idf_monitor (holding port) ---"
    trap 'echo "MONITOR-INTERRUPTED" >> "$log"; exit 0' INT
    i=0; while : ; do i=$((i+1)); echo "boot log line $i"; sleep 1; done ;;
  flash)
    [ -f "$fail" ] &&
      { echo "A fatal error occurred: Failed to connect to ESP32"; exit 2; }
    echo "Hash of data verified. Leaving..." ;;
  build) echo "Project build complete." ;;
  *)     echo "fake idf.py: $*" ;;
esac
