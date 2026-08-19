#!/bin/sh
# Shared helpers. Sourced by every script in this module; never run directly.
#
# Everything arrives in the environment, so nothing here parses JSON:
#   LUVUS_BIN_PATH         the running server's own binary. Use this, never a
#                          bare `luvus` on PATH, or a second install would talk
#                          to a different socket and report "no module".
#   LUVUS_MODULE_STATE_DIR a writable dir for this module's own bookkeeping
#   LUVUS_PANE_ID          the pane the action was invoked from (if any)
#   LUVUS_WORKSPACE_CWD    the folder of the node it was invoked against
#   LUVUS_SETTING_*        the declared settings

luvus="${LUVUS_BIN_PATH:-luvus}"
proj="${LUVUS_WORKSPACE_CWD:-$PWD}"
state="${LUVUS_MODULE_STATE_DIR:-/tmp}"

idf_path=$(printf '%s' "${LUVUS_SETTING_IDF_PATH:-$HOME/esp/esp-idf}" | sed "s|^~|$HOME|")
target="${LUVUS_SETTING_TARGET:-esp32s3}"
port="${LUVUS_SETTING_PORT:-}"
baud="${LUVUS_SETTING_BAUD:-460800}"
flash_method="${LUVUS_SETTING_FLASH_METHOD:-uart}"
auto_monitor="${LUVUS_SETTING_AUTO_MONITOR:-true}"

toast() { "$luvus" ui toast "$1"; }

# The chip this project is *actually* configured for, read from the sdkconfig
# `idf.py` generates. Empty when the project has never been configured.
#
# `$target` above is only the module setting — what a future `set-target` would
# apply. Showing that as if it were the project's chip is a guess presented as
# fact: the default is an arbitrary `esp32s3`, so an `esp32` project would have
# displayed the wrong chip with nothing to hint at it.
project_target() {
  [ -f "$proj/sdkconfig" ] || return 0
  sed -n 's/^CONFIG_IDF_TARGET="\(.*\)"$/\1/p' "$proj/sdkconfig" | head -1
}

# Fail early and visibly rather than typing a broken command into a pane.
require_idf() {
  if [ ! -f "$idf_path/export.sh" ]; then
    toast "ESP-IDF not found at $idf_path — set IDF_PATH in Settings -> Modules"
    exit 1
  fi
}

# The one place that knows how to enter the IDF environment. `-p`/`-b` are
# omitted when unset so idf.py can fall back to its own auto-detection.
idf_cmd() {
  _args=""
  [ -n "$port" ] && _args="$_args -p $port"
  [ -n "$baud" ] && _args="$_args -b $baud"
  printf 'cd %s && . %s/export.sh >/dev/null && idf.py%s %s' \
    "$(quote "$proj")" "$(quote "$idf_path")" "$_args" "$*"
}

quote() { printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"; }

# Where should a command run? The pane the user right-clicked, if there is one;
# otherwise split a fresh pane so a build never takes over an agent's pane.
target_pane() {
  if [ -n "${LUVUS_PANE_ID:-}" ]; then
    printf '%s' "$LUVUS_PANE_ID"
  else
    "$luvus" pane split 2>/dev/null | sed -n 's/.*"pane": *"\([0-9]*\)".*/\1/p' | head -1
  fi
}
