#!/bin/sh
# Runs *as* the pane's own process (a `[[panes]]` entrypoint), not typed into
# somebody else's shell.
#
# menuconfig is a full-screen terminal UI. Typing it into the pane you happened
# to right-click would take over whatever was there — quite possibly an agent
# mid-task. Its own tab gives it the screen it expects, and closing the tab
# closes menuconfig.
#
# Nothing here uses `exec`, and every failure path pauses. A pane that dies on
# error just blinks a tab out of existence with the reason gone with it.
set -u

pause() { echo; printf 'Press enter to close this tab. '; read -r _ || true; }

idf_path=$(printf '%s' "${LUVUS_SETTING_IDF_PATH:-$HOME/esp/esp-idf}" | sed "s|^~|$HOME|")
proj="${LUVUS_WORKSPACE_CWD:-$PWD}"

if [ ! -f "$idf_path/export.sh" ]; then
  echo "ESP-IDF not found at: $idf_path"
  echo "Set IDF_PATH in Settings -> Modules -> ESP-IDF, then reopen this tab."
  pause; exit 1
fi

# An ESP-IDF project is a CMake project with a top-level CMakeLists.txt. Saying
# so plainly beats idf.py's own error, which arrives after a slow startup.
if [ ! -f "$proj/CMakeLists.txt" ]; then
  echo "Not an ESP-IDF project: $proj"
  echo "(no CMakeLists.txt here)"
  echo
  echo "Open a node at your firmware folder and run menuconfig from there,"
  echo "or create one:  idf.py create-project my-app"
  pause; exit 1
fi

cd "$proj" || { echo "cannot enter $proj"; pause; exit 1; }
# shellcheck disable=SC1091
. "$idf_path/export.sh" >/dev/null 2>&1 || { echo "export.sh failed"; pause; exit 1; }

idf.py menuconfig || { echo; echo "menuconfig exited with an error."; pause; }
