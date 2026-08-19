#!/usr/bin/env python3
"""Post a webhook when an agent goes blocked or finishes.

luvus runs this as a plain subprocess, so there is no SDK to import. Two ways in:

  * the flat vars, easiest from any language:
      LUVUS_PANE_ID / LUVUS_PANE_AGENT / LUVUS_PANE_STATUS
      LUVUS_WORKSPACE_CWD, LUVUS_SETTING_WEBHOOK, LUVUS_SETTING_NOTIFY_ON, ...
  * the full snapshot, when you want more:
      LUVUS_MODULE_CONTEXT_JSON, and for an event hook LUVUS_MODULE_EVENT_JSON

Talk back to luvus through LUVUS_BIN_PATH, never a bare `luvus` on PATH -- that
keeps the module working on Windows named pipes as well as Unix sockets.
"""

import json
import os
import subprocess
import sys
import urllib.error
import urllib.request

LUVUS = os.environ.get("LUVUS_BIN_PATH", "luvus")


def luvus(*args: str) -> None:
    """Call back into luvus, ignoring failures (a module must never wedge the UI)."""
    try:
        subprocess.run([LUVUS, *args], check=False, capture_output=True, timeout=10)
    except (OSError, subprocess.SubprocessError):
        pass


def main() -> int:
    forced = "--force" in sys.argv

    # Settings arrive pre-resolved: manifest defaults with the user's choices on
    # top, already type-checked and clamped by luvus.
    webhook = os.environ.get("LUVUS_SETTING_WEBHOOK", "").strip()
    notify_on = os.environ.get("LUVUS_SETTING_NOTIFY_ON", "blocked")
    want_toast = os.environ.get("LUVUS_SETTING_TOAST", "true") == "true"

    agent = os.environ.get("LUVUS_PANE_AGENT") or "agent"
    status = os.environ.get("LUVUS_PANE_STATUS") or "unknown"
    pane = os.environ.get("LUVUS_PANE_ID") or "?"

    # An event hook prefers the event payload, which describes the pane that
    # actually changed rather than the one in focus.
    raw = os.environ.get("LUVUS_MODULE_EVENT_JSON")
    if raw:
        try:
            event = json.loads(raw)
            agent = event.get("agent") or agent
            status = event.get("status") or status
            pane = event.get("pane") or pane
        except json.JSONDecodeError:
            pass

    # Right-click always pings; an event only pings for the states asked for.
    if not forced:
        wanted = {"both": {"blocked", "done"}}.get(notify_on, {notify_on})
        if status not in wanted:
            return 0

    where = os.path.basename(os.environ.get("LUVUS_WORKSPACE_CWD", "") or "") or "luvus"
    message = f"{agent} is {status} in {where} (pane {pane})"

    if want_toast:
        luvus("ui", "toast", message)

    if not webhook:
        # Nothing configured yet: point the user at where to set it, once.
        if forced:
            luvus("ui", "toast", "set a Webhook URL in Settings > Modules")
        return 0

    body = json.dumps({"text": message}).encode()
    request = urllib.request.Request(
        webhook, data=body, headers={"Content-Type": "application/json"}
    )
    try:
        urllib.request.urlopen(request, timeout=10).close()
    except (urllib.error.URLError, OSError) as err:
        # stderr lands in `luvus module log`, which is where you debug a module.
        print(f"webhook failed: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
