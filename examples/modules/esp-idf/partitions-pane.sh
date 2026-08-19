#!/bin/sh
# Edit the project's partition table in your own editor, in its own tab.
#
# The IDE ships a form for this; the file itself is a CSV, so a terminal already
# has a better editor for it than any form — yours, with your keybindings.
# Runs as the pane's process (a `[[panes]]` entrypoint) so a full-screen editor
# gets the whole tab, and never uses `exec`: a pane that dies on error just
# blinks the tab out of existence with the reason gone.
set -u

pause() { echo; printf 'Press enter to close this tab. '; read -r _ || true; }

idf_path=$(printf '%s' "${LUVUS_SETTING_IDF_PATH:-$HOME/esp/esp-idf}" | sed "s|^~|$HOME|")
proj="${LUVUS_WORKSPACE_CWD:-$PWD}"
csv="$proj/partitions.csv"

if [ ! -f "$proj/CMakeLists.txt" ]; then
  echo "Not an ESP-IDF project: $proj"; pause; exit 1
fi

# A project only has its own CSV once it opts out of the built-in tables, so
# offer to start from the stock single-app layout rather than an empty file.
if [ ! -f "$csv" ]; then
  tpl="$idf_path/components/partition_table/partitions_singleapp.csv"
  echo "This project has no partitions.csv, so it uses a built-in table."
  echo
  echo "Creating one lets you customise the layout. You will also need to set"
  echo "  Partition Table -> Custom partition table CSV   in menuconfig."
  echo
  [ -f "$tpl" ] || { echo "No template found at $tpl"; pause; exit 1; }
  printf 'Create partitions.csv from the default template? [y/N] '
  read -r ans || ans=n
  case "$ans" in
    y|Y) cp "$tpl" "$csv" && echo "created $csv" ;;
    *)   echo "left unchanged."; pause; exit 0 ;;
  esac
fi

editor="${EDITOR:-}"
[ -n "$editor" ] || for e in nvim vim nano vi; do
  command -v "$e" >/dev/null 2>&1 && { editor="$e"; break; }
done
[ -n "$editor" ] || { echo "No editor found. Set \$EDITOR."; pause; exit 1; }

cd "$proj" || { echo "cannot enter $proj"; pause; exit 1; }
$editor "$csv" || { echo; echo "$editor exited with an error."; pause; }
