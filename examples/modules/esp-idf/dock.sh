#!/bin/sh
# The module's single sidebar dock: the boards it can see, then the commands
# that act on the selected one.
#
# One module, one dock. Splitting devices and commands apart meant two sections
# competing for the same sidebar height, and `dock_slots` divides that height
# between every mounted dock — so a second dock does not add space, it takes it
# from the first.
#
# Commands are grouped and **expand like the FILES tree**: click `▸ build` to
# open it, click again to close. `idf.py` has 67 subcommands and a sidebar is
# ~24 columns wide, so a row per command would bury the boards. A group is a
# folder, a board is a file.
#
# Only one group is open at a time. That is not a style choice: `draw_module_dock`
# renders `rows.iter().take(cap)` with no scrolling, so rows past the dock's
# height are silently dropped. An accordion keeps the list inside the slot; two
# open groups would push the boards off the bottom with nothing to show for it.
set -eu
. "$(dirname "$0")/lib.sh"

open_group=$(cat "$state/expanded" 2>/dev/null || printf '')

esc() { printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'; }

# A group header. Clicking it toggles; the chevron matches the FILES tree.
group() { # id, label
  if [ "$open_group" = "$1" ]; then g="▾"; else g="▸"; fi
  printf '{"text":"%s %s","action":"toggle","value":"%s"},' \
    "$g" "$(esc "$2")" "$(esc "$1")"
}
# A child row, indented under its open group.
child() { # label, action, value
  printf '{"text":"    %s","action":"%s","value":"%s"},' \
    "$(esc "$1")" "$2" "$(esc "${3:-}")"
}
open() { [ "$open_group" = "$1" ]; }
plain() { printf '{"text":"%s"},' "$(esc "$1")"; }

{
  printf '['

  # ── boards ────────────────────────────────────────────────────────────────
  # A board is a leaf, like a file: clicking it selects it rather than opening
  # anything. Scanning /dev keeps this dependency-free — esptool's port listing
  # differs between versions, and a missing esptool would leave an empty dock
  # with no visible reason.
  found=0
  for dev in /dev/cu.usbmodem* /dev/cu.usbserial* /dev/cu.SLAB* /dev/cu.wchusb* \
             /dev/ttyUSB* /dev/ttyACM*; do
    [ -e "$dev" ] || continue
    found=1
    if [ "$dev" = "$port" ]; then dot=done; else dot=idle; fi
    printf '{"text":"%s","action":"select-device","value":"%s","dot":"%s"},' \
      "$(esc "$(basename "$dev")")" "$(esc "$dev")" "$dot"
  done
  [ "$found" = 1 ] || plain "no board detected"

  # ── commands ──────────────────────────────────────────────────────────────
  # Listed with no board attached too: build, configure and clean do not need
  # one. Every child rides the `run` action with its subcommand as the value, so
  # adding one is a single line here rather than a new action in the manifest.
  plain ""

  # The chip row. It names what it is showing, and it shows what the *project*
  # is configured for rather than the module setting — with the dot carrying
  # whether those agree:
  #   done    the project is built for this chip
  #   blocked the setting differs; clicking applies it (`set-target` wipes build/)
  #   idle    never configured, so the setting is what a build would use
  now=$(project_target)
  if [ -z "$now" ]; then
    printf '{"text":"chip · %s","action":"set-target","dot":"idle"},' "$(esc "$target")"
  elif [ "$now" = "$target" ]; then
    printf '{"text":"chip · %s","action":"set-target","dot":"done"},' "$(esc "$now")"
  else
    printf '{"text":"chip · %s → %s","action":"set-target","dot":"blocked"},' \
      "$(esc "$now")" "$(esc "$target")"
  fi

  group build "build"
  if open build; then
    child "build"             run build
    child "app only"          run app
    child "bootloader"        run bootloader
    child "partition table"   run partition-table
    child "reconfigure"       run reconfigure
    child "clean"             run clean
    child "full clean"        run fullclean
  fi

  group flash "flash + monitor"
  if open flash; then
    child "flash + monitor"   flash
    child "flash only"        run flash
    child "app only (fast)"   run app-flash
    child "bootloader only"   run bootloader-flash
    child "merge into one"    run merge-bin
    child "UF2 image"         run uf2
    child "erase flash"       erase
  fi

  group monitor "monitor"
  if open monitor; then
    child "open monitor"      monitor
    child "print core dump"   run coredump-info
    child "debug: OpenOCD"    run openocd
    child "debug: GDB"        run gdb
  fi

  group configure "configure"
  if open configure; then
    child "menuconfig"        menuconfig
    child "set chip target"   set-target
    child "edit partitions"   edit-partitions
    child "print partitions"  run partition-table
  fi

  group size "size"
  if open size; then
    child "total"             run size
    child "by component"      run size-components
    child "by source file"    run size-files
    child "eFuse summary"     run efuse-summary
  fi

  printf ']'
} | sed 's/,]/]/g' | xargs -0 "$luvus" ui dock push --id esp-idf --title "ESP-IDF" --rows
