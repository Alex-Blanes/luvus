//! The bundled **agent skill** (docs): instructions that teach a coding agent
//! to delegate to other agents and control luvus over the local CLI.
//!
//! Two delivery shapes, because agents differ:
//! - **Claude Code** loads *skills* by relevance, so it gets `SKILL.md` and its
//!   on-demand reference under `~/.claude/skills/luvus/` (auto-triggered, low
//!   cost).
//! - **Codex** and **opencode** read an always-on `AGENTS.md`, so they get a
//!   short, delimited pointer block instead of the whole skill (kept small
//!   because it is always in context). The block never touches the user's own
//!   content in that file.
//!
//! All of it is compiled in via `include_str!`, printed by `luvus skill`,
//! installed by `luvus skill install`, and auto-installed on startup unless
//! `config.install_agent_skill` is off. Each target installs only when that
//! agent is actually set up on the machine.

use std::path::{Path, PathBuf};

/// The full skill text compiled into this binary: the release default, and the
/// fallback when no OTA update has been fetched.
pub const SKILL: &str = include_str!("../skills/luvus/SKILL.md");

/// Optional command index installed beside [`SKILL`] by current releases. The
/// primary skill keeps the complete targeting and safety contract because
/// the OTA updater shipped with Bohay 0.10.2 can download only `SKILL.md`.
pub const ADVANCED_CONTROL: &str = include_str!("../skills/luvus/references/advanced-control.md");

/// The skill text actually in use: the OTA-updated copy in the managed cache
/// (`~/.luvus/skill/SKILL.md`, written by `luvus skill update`) when present and
/// valid, else the compiled-in [`SKILL`]. This is what lets a skill fix reach
/// users between releases; a missing, empty, or garbled cache falls back safely.
pub fn effective_skill() -> std::borrow::Cow<'static, str> {
    let cached = crate::persist::skill_dir().join("SKILL.md");
    match std::fs::read_to_string(&cached) {
        Ok(s) if skill_valid(&s) => std::borrow::Cow::Owned(s),
        _ => std::borrow::Cow::Borrowed(SKILL),
    }
}

/// Cheap sanity check for a skill download: the luvus YAML frontmatter, unified
/// delegation and control markers, and a sensible size. This rejects stale `$`
/// delegation skills, 404 pages, and truncated downloads before replacement.
pub fn skill_valid(s: &str) -> bool {
    let t = s.trim_start();
    t.starts_with("---")
        && t.contains("name: luvus")
        && t.contains("=target")
        && t.contains("agent send")
        && (400..200_000).contains(&s.len())
}

/// Save an OTA-fetched skill into the managed cache (`~/.luvus/skill/SKILL.md`),
/// creating the dir. Returns the cache path. `effective_skill` reads it back.
pub fn save_managed(text: &str) -> std::io::Result<PathBuf> {
    let dir = crate::persist::skill_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("SKILL.md");
    std::fs::write(&path, text)?;
    Ok(path)
}

// ── full skill (Claude Code) ────────────────────────────────────────────────

/// Write the skill and advanced reference under `<dir>`. Returns the skill path.
pub fn install_to(dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("SKILL.md");
    std::fs::write(&path, effective_skill().as_bytes())?;
    let references = dir.join("references");
    std::fs::create_dir_all(&references)?;
    std::fs::write(
        references.join("advanced-control.md"),
        ADVANCED_CONTROL.as_bytes(),
    )?;
    Ok(path)
}

/// Remove the skill and advanced reference, then empty directories. Returns
/// whether either managed file was actually removed.
pub fn uninstall_from(dir: &Path) -> std::io::Result<bool> {
    let path = dir.join("SKILL.md");
    let reference = dir.join("references").join("advanced-control.md");
    let existed = path.exists() || reference.exists();
    if !existed {
        return Ok(false);
    }
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    if reference.exists() {
        std::fs::remove_file(&reference)?;
    }
    let _ = std::fs::remove_dir(dir.join("references"));
    let _ = std::fs::remove_dir(dir); // best effort: drop the now-empty skill dir
    Ok(true)
}

/// Claude Code's skills dir for luvus, only when Claude Code is set up
/// (`~/.claude` exists), so luvus never creates that tree for a non-user.
fn claude_skill_dir() -> Option<PathBuf> {
    let claude = crate::platform::home_dir()?.join(".claude");
    claude.is_dir().then(|| claude.join("skills").join("luvus"))
}

// ── short pointer (AGENTS.md agents: Codex, opencode) ───────────────────────

const POINTER_BEGIN: &str = "<!-- BEGIN luvus (managed by luvus; do not edit inside) -->";
const POINTER_END: &str = "<!-- END luvus -->";
const LEGACY_POINTER_BEGIN: &str = "<!-- BEGIN bohay (managed by bohay; do not edit inside) -->";
const LEGACY_POINTER_END: &str = "<!-- END bohay -->";

/// The always-on pointer body: a one-liner that points at the single source of
/// truth (`luvus skill`), so nothing here can drift from `SKILL.md`. Kept tiny
/// because an `AGENTS.md` is in the agent's context on every turn.
const POINTER_BODY: &str = "\
## luvus: delegate and control the current session

When `$LUVUS_ENV` is `1` you are inside luvus and may control its workspaces, tabs, panes, and agents when asked. A line beginning `=target message` delegates that message to the named agent, pane id, or unique agent kind. Run `luvus skill` to learn how. Never delegate unless the user asks.";

/// The full managed block as it appears in an `AGENTS.md`.
fn pointer_block() -> String {
    format!("{POINTER_BEGIN}\n{POINTER_BODY}\n{POINTER_END}\n")
}

/// `s` with any existing managed block removed (and the blank space it left).
fn strip_block(s: &str) -> String {
    let without_current = strip_one_block(s, POINTER_BEGIN, POINTER_END);
    strip_one_block(&without_current, LEGACY_POINTER_BEGIN, LEGACY_POINTER_END)
}

fn strip_one_block(s: &str, begin: &str, end_marker: &str) -> String {
    let (Some(b), Some(e)) = (s.find(begin), s.find(end_marker)) else {
        return s.to_string();
    };
    if e < b {
        return s.to_string();
    }
    let end = e + end_marker.len();
    let head = s[..b].trim_end_matches(['\n', ' ']);
    let tail = s[end..].trim_start_matches('\n');
    match (head.is_empty(), tail.is_empty()) {
        (true, _) => tail.to_string(),
        (false, true) => format!("{head}\n"),
        (false, false) => format!("{head}\n\n{tail}"),
    }
}

/// Upsert the managed block into `file` (an `AGENTS.md`), leaving the user's own
/// content intact. Returns `true` if the file changed.
fn upsert_pointer(file: &Path) -> std::io::Result<bool> {
    let existing = std::fs::read_to_string(file).unwrap_or_default();
    let base = strip_block(&existing);
    let updated = if base.trim().is_empty() {
        pointer_block()
    } else {
        format!("{}\n\n{}", base.trim_end(), pointer_block())
    };
    if updated == existing {
        return Ok(false);
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, updated)?;
    Ok(true)
}

/// Remove the managed block from `file`, keeping everything else. Returns `true`
/// if the file changed.
fn remove_pointer(file: &Path) -> std::io::Result<bool> {
    let Ok(existing) = std::fs::read_to_string(file) else {
        return Ok(false);
    };
    let updated = strip_block(&existing);
    if updated == existing {
        return Ok(false);
    }
    std::fs::write(file, updated)?;
    Ok(true)
}

/// Codex's global `AGENTS.md`, when Codex is set up (`$CODEX_HOME` or `~/.codex`).
fn codex_agents_file() -> Option<PathBuf> {
    let dir = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            crate::platform::home_dir()
                .unwrap_or_default()
                .join(".codex")
        });
    dir.is_dir().then(|| dir.join("AGENTS.md"))
}

/// opencode's global `AGENTS.md`, when opencode is set up
/// (`$XDG_CONFIG_HOME/opencode` or `~/.config/opencode`).
fn opencode_agents_file() -> Option<PathBuf> {
    let cfg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            crate::platform::home_dir()
                .unwrap_or_default()
                .join(".config")
        });
    let dir = cfg.join("opencode");
    dir.is_dir().then(|| dir.join("AGENTS.md"))
}

// ── aggregate install / uninstall (all set-up agents) ───────────────────────

/// Install/refresh the skill for every supported agent that is set up on this
/// machine: the full skill for Claude Code, a pointer block for Codex and
/// opencode. Idempotent (a no-op where already current), so it is cheap to call
/// on every startup. Returns the files it wrote.
pub fn install_default() -> Vec<PathBuf> {
    let mut done = Vec::new();
    if let Some(dir) = claude_skill_dir() {
        let legacy = dir.parent().unwrap_or(&dir).join("bohay");
        let _ = uninstall_from(&legacy);
        let path = dir.join("SKILL.md");
        let eff = effective_skill();
        let reference = dir.join("references").join("advanced-control.md");
        let current = std::fs::read_to_string(&path).ok().as_deref() == Some(eff.as_ref())
            && std::fs::read_to_string(reference).ok().as_deref() == Some(ADVANCED_CONTROL);
        if current {
            done.push(path);
        } else if let Ok(p) = install_to(&dir) {
            done.push(p);
        }
    }
    for file in [codex_agents_file(), opencode_agents_file()]
        .into_iter()
        .flatten()
    {
        if upsert_pointer(&file).is_ok() {
            done.push(file);
        }
    }
    done
}

/// Remove the skill/pointer from every supported agent's location. Returns the
/// files it changed.
pub fn uninstall_default() -> Vec<PathBuf> {
    let mut done = Vec::new();
    if let Some(dir) = claude_skill_dir() {
        if uninstall_from(&dir).unwrap_or(false) {
            done.push(dir.join("SKILL.md"));
        }
        let legacy = dir.parent().unwrap_or(&dir).join("bohay");
        if uninstall_from(&legacy).unwrap_or(false) {
            done.push(legacy.join("SKILL.md"));
        }
    }
    for file in [codex_agents_file(), opencode_agents_file()]
        .into_iter()
        .flatten()
    {
        if remove_pointer(&file).unwrap_or(false) {
            done.push(file);
        }
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_to_writes_the_bundled_skill() {
        // Isolate the home so no real OTA cache is picked up: with none present,
        // the effective skill is the compiled-in default.
        let _env = crate::persist::test_env("skill-install");
        let dir = std::env::temp_dir().join("luvus-skill-install-test");
        let _ = std::fs::remove_dir_all(&dir);
        let path = install_to(&dir).expect("install");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SKILL);
        assert_eq!(
            std::fs::read_to_string(dir.join("references/advanced-control.md")).unwrap(),
            ADVANCED_CONTROL
        );
        // Sanity: this is the unified control skill, not an empty file.
        assert!(
            SKILL.contains("agent send")
                && SKILL.contains("workspace list")
                && SKILL.contains("=target")
                && SKILL.contains("name: luvus")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn codex_plugin_mirrors_the_bundled_skill() {
        assert_eq!(
            include_str!("../plugins/luvus/skills/luvus/SKILL.md"),
            SKILL
        );
        assert_eq!(
            include_str!("../plugins/luvus/skills/luvus/references/advanced-control.md"),
            ADVANCED_CONTROL
        );
    }

    #[test]
    fn bundled_and_plugin_skills_document_layout_controls() {
        let plugin = include_str!("../plugins/luvus/skills/luvus/SKILL.md");
        let manifest = include_str!("../plugins/luvus/.codex-plugin/plugin.json");
        let metadata = include_str!("../plugins/luvus/skills/luvus/agents/openai.yaml");
        let guide = include_str!("../website/src/content/docs/docs/guides/codex-plugin.mdx");
        let required = [
            "luvus pane move <pane-id> --tab <tab-number>",
            "luvus pane move <pane-id> --new-tab",
            "luvus tab move <from> <to>",
            "Tab numbers are 1-based",
            "luvus workspace rename <workspace-index> <name>",
            "luvus workspace pin <workspace-index>",
            "luvus workspace unpin <workspace-index>",
            "stable 0-based",
            "do not run `help` before these documented",
        ];
        for marker in required {
            assert!(
                SKILL.contains(marker),
                "bundled skill is missing layout command marker: {marker}"
            );
            assert!(
                plugin.contains(marker),
                "Codex plugin skill is missing layout command marker: {marker}"
            );
        }
        assert!(
            ADVANCED_CONTROL.contains("luvus pane move <id>"),
            "advanced control reference is missing pane move safety guidance"
        );
        assert!(
            ADVANCED_CONTROL.contains("move <from> <to>")
                && ADVANCED_CONTROL.contains("tab positions are 1-based"),
            "advanced control reference is missing tab move safety guidance"
        );
        assert!(
            ADVANCED_CONTROL.contains("luvus workspace rename <i> <name>")
                && ADVANCED_CONTROL.contains("pinning changes")
                && ADVANCED_CONTROL.contains("only display order")
                && ADVANCED_CONTROL.contains("recover from `not_found`"),
            "advanced control reference is missing workspace organization guidance"
        );
        assert!(
            manifest.contains("\"version\": \"0.3.0\"")
                && manifest.contains("Rename and pin a Luvus workspace"),
            "Codex plugin manifest is missing workspace organization metadata"
        );
        assert!(
            metadata.contains("Use $luvus to rename and pin a workspace"),
            "Codex skill metadata is missing workspace organization guidance"
        );
        assert!(
            guide.contains("luvus workspace rename 2") && guide.contains("display_position"),
            "Codex plugin guide is missing workspace organization guidance"
        );
    }

    #[test]
    fn codex_plugin_documents_native_agent_forks() {
        let plugin = include_str!("../plugins/luvus/skills/luvus/SKILL.md");
        let manifest = include_str!("../plugins/luvus/.codex-plugin/plugin.json");
        let guide = include_str!("../website/src/content/docs/docs/guides/codex-plugin.mdx");

        let skill_markers = [
            "luvus agent get <target>",
            "luvus agent fork <target> [--name <alias>] [--no-focus]",
            "Native forks currently support Claude, Codex, and Pi",
            "`unsupported_agent`, `session_unknown`, or `spawn_failed`",
            "Do not approximate a failed fork",
        ];
        for marker in skill_markers {
            assert!(
                SKILL.contains(marker),
                "bundled skill is missing native fork guidance: {marker}"
            );
            assert!(
                plugin.contains(marker),
                "Codex plugin skill is missing native fork guidance: {marker}"
            );
        }

        assert!(
            ADVANCED_CONTROL.contains("luvus agent fork"),
            "advanced control reference is missing agent fork safety guidance"
        );
        assert!(manifest.contains("Fork a supported live Luvus agent"));
        assert!(guide.contains("### Fork an agent session"));
        assert!(guide.contains("luvus agent fork reviewer --name experiment --no-focus"));
    }

    #[test]
    fn bundled_and_plugin_skills_delegate_without_a_preflight_list() {
        let plugin = include_str!("../plugins/luvus/skills/luvus/SKILL.md");
        let guide = include_str!("../website/src/content/docs/docs/guides/codex-plugin.mdx");
        let required = [
            "luvus agent send <target> \"<message>\"` directly",
            "Do not run `agent list` first",
            "accept the returned pane, agent, name, and status",
            "Only after `not_found` or `ambiguous_target`",
            "run `luvus agent list` once",
            "Never absorb or",
            "perform the delegated task locally after delivery fails",
        ];
        for marker in required {
            assert!(
                SKILL.contains(marker),
                "bundled skill is missing direct delegation guidance: {marker}"
            );
            assert!(
                plugin.contains(marker),
                "Codex plugin skill is missing direct delegation guidance: {marker}"
            );
        }
        assert!(!SKILL.contains("Run `luvus agent list` against the selected session"));
        assert!(!plugin.contains("Run `luvus agent list` against the selected session"));
        assert!(guide.contains("the plugin does not prepend `agent list`"));
        assert!(guide.contains("once only after `not_found` or `ambiguous_target`"));
    }

    #[test]
    fn bundled_and_plugin_skills_explain_how_to_install_a_missing_client() {
        let plugin = include_str!("../plugins/luvus/skills/luvus/SKILL.md");
        let required = [
            "Only after an attempted Luvus action cannot run because command lookup finds",
            "no `luvus` client, report that Luvus is not installed and stop",
            "curl -fsSL https://luvus.dev/install.sh | sh",
            "brew install RizRiyz/luvus/luvus",
            "cargo install luvus",
            "Do not show this",
            "preemptively or for socket, permission, server, or other command",
            "failures",
        ];
        for marker in required {
            assert!(
                SKILL.contains(marker),
                "bundled skill is missing installation guidance: {marker}"
            );
            assert!(
                plugin.contains(marker),
                "Codex plugin skill is missing installation guidance: {marker}"
            );
        }
    }

    #[test]
    fn bundled_and_plugin_skills_require_explicit_luvus_intent() {
        let plugin = include_str!("../plugins/luvus/skills/luvus/SKILL.md");
        let required = [
            "Use only for a line beginning with `=target message`",
            "an explicit request naming Luvus",
            "a request to delegate to a named live Luvus agent or pane",
            "Do not use for ordinary coding, file edits, Git operations, tests",
            "unless the user explicitly connects the request to Luvus",
            "Being inside Luvus does not trigger this skill by itself",
        ];
        for marker in required {
            assert!(
                SKILL.contains(marker),
                "bundled skill is missing explicit trigger guidance: {marker}"
            );
            assert!(
                plugin.contains(marker),
                "Codex plugin skill is missing explicit trigger guidance: {marker}"
            );
        }
        assert!(
            !SKILL.contains("Also use when asked to delegate work, inspect or control Luvus"),
            "bundled skill still has the broad automatic trigger"
        );
        assert!(
            !plugin.contains("Also use when asked to delegate work, inspect or control Luvus"),
            "Codex plugin skill still has the broad automatic trigger"
        );
    }

    #[test]
    fn skill_valid_accepts_a_real_skill_and_rejects_junk() {
        assert!(skill_valid(SKILL), "the bundled skill validates");
        assert!(!skill_valid(""), "empty is rejected");
        assert!(
            !skill_valid("<html>404 Not Found</html>"),
            "a 404 page is rejected"
        );
        assert!(
            !skill_valid("---\nname: something-else\n---\nbody"),
            "wrong frontmatter is rejected"
        );
    }

    #[test]
    fn skill_is_self_contained_for_v0102_single_file_updates() {
        // Bohay 0.10.2 fetches and installs SKILL.md only. Keep every required
        // advanced read route and the mutation safety contract in that file so
        // a current skill remains complete even when no reference is present.
        let required = [
            "luvus files tree",
            "luvus git status",
            "luvus worktree list",
            "luvus task list",
            "luvus lease list",
            "luvus module list",
            "luvus ui dock list",
            "Removal requires",
            "explicit authorization",
            "never retain an unbounded stream",
            "Its absence is not a blocker",
        ];
        for marker in required {
            assert!(
                SKILL.contains(marker),
                "single-file skill is missing compatibility marker: {marker}"
            );
        }

        // Match the validator shipped in Bohay v0.10.2. If this fails, that release
        // would reject the current main-branch skill before installing it.
        let trimmed = SKILL.trim_start();
        assert!(trimmed.starts_with("---"));
        assert!(trimmed.contains("name: luvus"));
        assert!((400..200_000).contains(&SKILL.len()));
    }

    #[test]
    fn effective_skill_prefers_the_managed_cache() {
        let _env = crate::persist::test_env("skill-ota");
        let _ = std::fs::remove_dir_all(crate::persist::skill_dir()); // fresh: no leftover cache
                                                                      // No cache yet -> the compiled-in default.
        assert_eq!(effective_skill(), SKILL);

        // An OTA'd (valid) skill in the managed cache wins, and install_to writes
        // it instead of the default.
        let ota = format!(
            "---\nname: luvus\ndescription: updated\n---\n\n# luvus\n\n=target delegates with agent send.\n{}",
            "Body long enough to clear the size floor. ".repeat(12)
        );
        let ota = ota.as_str();
        let cache = save_managed(ota).expect("save managed");
        assert!(cache.ends_with("SKILL.md"));
        assert_eq!(effective_skill(), ota);

        let dir = std::env::temp_dir().join("luvus-skill-ota-install");
        let _ = std::fs::remove_dir_all(&dir);
        let path = install_to(&dir).expect("install");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), ota);
        let _ = std::fs::remove_dir_all(&dir);

        // A garbled cache is ignored -> fall back to the default (never break).
        save_managed("not a skill").unwrap();
        assert_eq!(effective_skill(), SKILL);
    }

    #[test]
    fn uninstall_removes_the_skill_and_is_a_noop_when_absent() {
        let dir = std::env::temp_dir().join("luvus-skill-uninstall-test");
        let _ = std::fs::remove_dir_all(&dir);
        install_to(&dir).unwrap();
        assert!(uninstall_from(&dir).unwrap(), "removed an installed skill");
        assert!(!dir.join("SKILL.md").exists());
        assert!(!dir.join("references/advanced-control.md").exists());
        assert!(
            !uninstall_from(&dir).unwrap(),
            "no-op when nothing is there to remove"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pointer_upsert_preserves_user_content_and_is_idempotent() {
        let dir = std::env::temp_dir().join("luvus-pointer-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("AGENTS.md");
        std::fs::write(&file, "# My rules\n\nBe concise.\n").unwrap();

        assert!(upsert_pointer(&file).unwrap(), "first upsert writes");
        let after = std::fs::read_to_string(&file).unwrap();
        assert!(after.contains("# My rules"), "user content kept");
        assert!(after.contains(POINTER_BEGIN), "managed block added");
        assert!(
            after.contains("luvus skill"),
            "points at the source of truth"
        );
        assert!(after.contains("=target"), "documents delegation syntax");

        // Running again changes nothing.
        assert!(!upsert_pointer(&file).unwrap(), "second upsert is a no-op");

        // Removing the block restores the user's content without the pointer.
        assert!(remove_pointer(&file).unwrap(), "remove changes the file");
        let cleaned = std::fs::read_to_string(&file).unwrap();
        assert!(cleaned.contains("# My rules"));
        assert!(!cleaned.contains(POINTER_BEGIN));
        assert!(!remove_pointer(&file).unwrap(), "remove is a no-op after");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pointer_upsert_replaces_the_legacy_managed_block_only() {
        let dir = std::env::temp_dir().join("luvus-legacy-pointer-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("AGENTS.md");
        std::fs::write(
            &file,
            format!(
                "# My rules\n\n{LEGACY_POINTER_BEGIN}\nold managed text\n{LEGACY_POINTER_END}\n\nKeep me.\n"
            ),
        )
        .unwrap();

        assert!(upsert_pointer(&file).unwrap());
        let updated = std::fs::read_to_string(&file).unwrap();
        assert!(updated.contains("# My rules"));
        assert!(updated.contains("Keep me."));
        assert!(updated.contains(POINTER_BEGIN));
        assert!(!updated.contains(LEGACY_POINTER_BEGIN));
        assert!(!updated.contains("old managed text"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
