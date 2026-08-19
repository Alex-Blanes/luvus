// Open the module's `notes` pane next to the right-clicked pane, then
// (optionally) name the tab.
//
// luvus hands every command its context twice: as flat LUVUS_* vars for quick
// use, and as one LUVUS_MODULE_CONTEXT_JSON blob when you want the whole shape.
const { spawnSync } = require("node:child_process");

const luvus = process.env.LUVUS_BIN_PATH ?? "luvus";

function run(...args) {
  const r = spawnSync(luvus, args, { encoding: "utf8" });
  if (r.status !== 0) {
    // stderr shows up in `luvus module log`.
    process.stderr.write(r.stderr ?? `${args[0]} failed\n`);
  }
  return r;
}

const moduleId = process.env.LUVUS_MODULE_ID;

// `module pane open` spawns the manifest's `[[panes]]` entrypoint as a real
// pane. It is a normal luvus pane afterwards, so pane.focus/close/split all
// work on it like any other.
run("module", "pane", "open", moduleId, "notes", "--placement", "split");

if (process.env.LUVUS_SETTING_NAME_THE_TAB === "true") {
  // Tab index comes from the context; renaming is the same label the
  // tab-rename modal writes.
  const tab = process.env.LUVUS_TAB_INDEX ?? "";
  run("tab", "rename", "notes", ...(tab ? ["--tab", tab] : []));
}

run("ui", "toast", "notes opened");
