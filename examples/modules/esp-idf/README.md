# ESP-IDF

Build, flash and monitor ESP32 boards from luvus. No core changes: this is a
plain module — a manifest plus a few `sh` scripts that shell out to `idf.py`.

## Why this instead of `idf.py` in a pane

Because of one thing: **`idf.py monitor` holds the serial port.** To reflash you
have to stop the monitor, flash, and start it again — and you lose the log every
time. That is the loop an embedded developer runs dozens of times a day.

The **flash + monitor** action does it properly:

1. sends the monitor pane `Ctrl+C` (`0x03`) so the port is released
2. runs the flash **in that same pane**
3. reopens the monitor there

So what the board printed *before* the flash stays on screen directly above what
it prints *after*. Comparing those two is the actual work of debugging firmware,
and normally it is destroyed on every flash.

Everything else — build, menuconfig, size, erase — simply runs in a real pane.
`menuconfig` is already a terminal UI and `monitor` is a serial console, so
there is nothing to reimplement; an editor plugin has to rebuild both as GUIs,
and luvus does not.

## Install

```sh
luvus module link examples/modules/esp-idf
```

Then open **Settings → Modules → ESP-IDF** and set **IDF_PATH** to the folder
containing `export.sh` (usually `~/esp/esp-idf`). Pick your chip while you are
there.

## Use

An **ESP-IDF** dock appears in the left sidebar. Command groups **expand and
collapse like the FILES tree** — click a group to open it, click again to close:

```
ESP-IDF                        ESP-IDF
● cu.usbserial-110             ● cu.usbserial-110
● chip · esp32s3               ● chip · esp32s3
▸ build                        ▾ build
▸ flash + monitor                  build
▸ monitor                          app only
▸ configure                        bootloader
▸ size                             partition table
                                   reconfigure
                                   clean
                                   full clean
                               ▸ flash + monitor
                               ...
```

`idf.py` has 67 subcommands and a sidebar is about 24 columns wide, so a row per
command would bury the boards. A group is a folder; a board is a file.

| Group | Contains |
|---|---|
| `build` | build · app only · bootloader · partition table · reconfigure · clean · full clean |
| `flash + monitor` | flash + monitor · flash only · app only (fast) · bootloader only · merge-bin · UF2 · **erase flash** |
| `monitor` | open monitor · print core dump · OpenOCD · GDB |
| `configure` | menuconfig · set chip target · edit partitions · print partitions |
| `size` | total · by component · by source file · eFuse summary |

That is 29 commands behind 5 rows. Adding another is one line in `dock.sh`,
because every child rides the same `run` action with its subcommand as the
row's value.

**Only one group is open at a time.** That is not a style choice: module docks
render a fixed slice of their rows and do not scroll, so a second open group
would push the boards off the bottom with nothing on screen to say so.

### The chip row

`chip · esp32s3` names the chip the **project** is configured for, read from its
`sdkconfig` — not the value in Settings. The dot says whether those agree:

| Dot | Means |
|---|---|
| green | the project is built for this chip |
| red | `chip · esp32 → esp32s3` — Settings wants a different chip; click to apply |
| grey | never configured, so this is what a build would use |

Clicking it runs `idf.py set-target`, which **wipes the build directory** — so
it is a click you take deliberately, not something the module does behind you
when the setting changes.

### Boards

Click a board to make it the active port — the dot marks which is selected.
Every command runs against that port.

**`erase flash` is inside the `flash + monitor` group**, so reaching it takes
opening the group first. It is the one action here you cannot undo, and it
should not sit one stray click away in a collapsed sidebar.

Commands are typed into a real pane on purpose: a build you cannot `Ctrl+C` is
worse than no build button at all.

This module deliberately adds **nothing** to luvus's own pane or node right-click
menus, and uses no right-click menus of its own. It is a global module, so
menu entries would appear on every project whether or not it is firmware — and
expanding a group is discoverable in a way a hidden menu is not. Its dock is its
only surface.

## Settings

| Key | What |
|---|---|
| `idf_path` | folder containing `export.sh`; `~` is expanded |
| `target` | chip, applied by the "set chip target" action |
| `port` | set by clicking a DEVICES row; editable for ports the scan misses |
| `baud` | flash baud rate |
| `flash_method` | uart / dfu / jtag |
| `auto_monitor` | reopen the monitor after a flash |

Leaving `port` empty omits `-p`, so `idf.py` falls back to its own detection.

## Driving it from an agent

Every action is reachable over the socket, so an agent can close the loop on
real hardware:

```sh
luvus module run example.esp-idf build
luvus pane read            # did it compile?
luvus module run example.esp-idf flash
luvus pane read            # what did the board actually print?
```

## Testing it without a board

```sh
sh test/run.sh              # uses `luvus` from PATH
sh test/run.sh ./target/debug/luvus   # or point it at a specific build
```

Spins up a throwaway server against a fake `idf.py` and checks the whole flow:
the module registers, the dock mounts, `build` reaches `idf.py` with the right
`-p`/`-b`, the monitor really holds its pane, flashing interrupts it, the
monitor comes back, the pre-flash log survives, and a *failed* flash does **not**
bring the monitor back over the error. Nothing is left running afterwards.

With a real board attached, the things the fake cannot cover are: whether the
1-second wait after `Ctrl+C` is long enough for your adapter to release the
port, and whether your board needs the auto-reset that some USB-serial chips
miss.

## Limits, honestly

- **No graphical debugging.** Use `idf.py gdb` in a pane. If you want
  breakpoints in a gutter, use the IDE.
- **No NVS / partition GUI editors, no size charts.** `idf.py size` and
  `idf.py partition-table` print to a pane; edit the CSV in your editor.
- **No compiler-error list.** Build errors stay as text in the pane.
- Device discovery scans `/dev` rather than using `esptool.py`, so it is
  dependency-free but only finds USB serial adapters by their usual names.
