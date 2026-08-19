#!/bin/sh
# Fake idf.py for testing the module without hardware.
#   - logs every call to $IDF_FAKE_LOG (default /tmp/idf-calls.log)
#   - `monitor` blocks and streams, like the real one holding the port
#   - create /tmp/idf-fake-fail to make `flash` exit non-zero. A file, not an
#     env var: the fake runs inside a luvus-spawned pane, which does not inherit
#     the test shell's environment.
log="${IDF_FAKE_LOG:-/tmp/idf-calls.log}"
for a in "$@"; do sub="$a"; done          # subcommand is the LAST arg, after -p/-b
echo "IDFCALL $*" >> "$log"
case "$sub" in
  monitor)
    echo "--- idf_monitor (holding port) ---"
    trap 'echo "MONITOR-INTERRUPTED" >> "$log"; exit 0' INT
    i=0; while : ; do i=$((i+1)); echo "boot log line $i"; sleep 1; done ;;
  flash)
    [ -f /tmp/idf-fake-fail ] &&
      { echo "A fatal error occurred: Failed to connect to ESP32"; exit 2; }
    echo "Hash of data verified. Leaving..." ;;
  build) echo "Project build complete." ;;
  *)     echo "fake idf.py: $*" ;;
esac
