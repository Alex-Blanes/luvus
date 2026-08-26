//! Update checks and the explicit `luvus update` installer.
//!
//! Automatic checks remain notify-only: they fetch the small manifest on a
//! background thread and never mutate the installation. The explicit CLI
//! command checks first, then delegates to a detected package manager or
//! verifies a release archive before atomically replacing a direct install.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::event::AppEvent;

/// Where **this fork** publishes its own builds. Upstream's
/// `luvus.dev/latest.json` is deliberately not it: this binary carries local
/// work upstream never sees, and its semver stays whatever upstream release it
/// was merged from, so the feed is keyed on the fork build number instead.
const MANIFEST_URL: &str =
    "https://github.com/Alex-Blanes/luvus/releases/latest/download/latest.json";
/// Where the fork's release assets live, one directory per tag.
const RELEASE_BASE: &str = "https://github.com/Alex-Blanes/luvus/releases/download";
/// Upstream's release feed. Checked only to *say* a new upstream release exists
/// — nothing here can install it, because integrating upstream is a merge.
const UPSTREAM_RELEASES_URL: &str = "https://api.github.com/repos/RizRiyz/luvus/releases/latest";
/// This build's version (no leading `v`).
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// This build's fork build number ([`build.rs`]). `None` when the binary was
/// built without the `v<version>` tag in reach (a crates.io tarball, a shallow
/// clone) — there is nothing to compare then, so the fork check stays quiet
/// rather than nagging about an update it cannot reason about.
fn current_build() -> Option<u32> {
    env!("LUVUS_BUILD").parse().ok()
}

/// One published fork build. `version` is upstream's semver, carried so the
/// downloaded binary can still be verified by `--version`; `build` is what
/// actually decides newer; `tag` names the release its assets hang off.
struct ForkRelease {
    version: String,
    build: u32,
    tag: String,
}

impl ForkRelease {
    /// What the sidebar and the toasts show — the same shape `build.rs` bakes
    /// into `LUVUS_VERSION_LABEL`, so "available" and "installed" are comparable
    /// at a glance.
    fn label(&self) -> String {
        format!("{} - 0.{:02}", self.version, self.build)
    }
}

/// The manifest URL to check, honoring `$LUVUS_UPDATE_MANIFEST` — an override for
/// testing (point it at a local `file://…/latest.json` or a dev server to see the
/// indicator without deploying the site). Falls back to the production URL.
fn manifest_url() -> String {
    std::env::var("LUVUS_UPDATE_MANIFEST").unwrap_or_else(|_| MANIFEST_URL.to_string())
}

/// Upstream's feed, honoring `$LUVUS_UPSTREAM_RELEASES` for testing.
fn upstream_url() -> String {
    std::env::var("LUVUS_UPSTREAM_RELEASES").unwrap_or_else(|_| UPSTREAM_RELEASES_URL.to_string())
}

/// How often the background checker re-runs.
///
/// Deliberately not a day. The luvus **server outlives its windows** and can run
/// for weeks, so the check has to assume the release it is looking for will be
/// published *while the process is already running*, not before it started. At a
/// 24-hour interval a release cut twenty minutes after a server start stayed
/// invisible until the following day.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// Spawn the background checker: one check shortly after startup, then every
/// [`CHECK_EVERY`]. Sends [`AppEvent::UpdateAvailable`] only when the manifest
/// names a strictly newer release than this build.
pub fn spawn_check(tx: Sender<AppEvent>, auto_install: bool) {
    thread::spawn(move || {
        // A short initial delay so a launch is never slowed by a network call.
        thread::sleep(Duration::from_secs(5));
        loop {
            check_once(&tx, &manifest_url(), auto_install);
            check_upstream_once(&tx, &upstream_url());
            thread::sleep(CHECK_EVERY);
        }
    });
}

/// Check once, now, off the caller's thread.
///
/// The periodic check cannot help someone who has *just* upgraded elsewhere and
/// wants to know: waiting up to [`CHECK_EVERY`] to find out is the whole
/// complaint. Opening the changelog is exactly the moment the question is being
/// asked, so that asks again.
pub fn check_now(tx: Sender<AppEvent>, auto_install: bool) {
    thread::spawn(move || {
        check_once(&tx, &manifest_url(), auto_install);
        check_upstream_once(&tx, &upstream_url());
    });
}

/// What one check found. Only the *asked-for* check reports this.
///
/// The periodic check stays silent unless there is news, because a toast every
/// [`CHECK_EVERY`] saying "nothing changed" is noise nobody asked for. A press of
/// the changelog's **Check for updates** button is a question, and a question
/// that gets no answer reads as a broken button, so that path reports all three
/// outcomes. `Failed` is kept distinct from `Current` on purpose: telling someone
/// they are up to date when the network call actually failed is a lie.
pub enum CheckOutcome {
    Newer(String),
    Current,
    Failed,
}

/// One fetch-compare, with the answer handed back rather than swallowed.
fn fetch_outcome(url: &str) -> CheckOutcome {
    match fetch_release(url) {
        Some(release) if is_newer_build(&release) => CheckOutcome::Newer(release.label()),
        Some(_) => CheckOutcome::Current,
        None => CheckOutcome::Failed,
    }
}

fn fetch_release(url: &str) -> Option<ForkRelease> {
    http_get(url).as_deref().and_then(parse_manifest)
}

/// Is `release` newer than what is running? Semver first, build number only as
/// the tiebreaker within one release.
///
/// The build number counts commits since the `v<version>` tag, so **it resets
/// every time the fork merges a new upstream release** — the count starts again
/// from the new tag. Comparing the two numbers alone therefore reads a version
/// bump as going backwards: at the 0.12.0 → 0.13.1 merge the running build was
/// 94 and the first build of the newer release was 68, so `68 > 94` was false
/// and the updater refused that release, and every release after it, until the
/// fork happened to accumulate 27 more commits. Nothing looked wrong — the label
/// still built, the check still ran, and it reported being up to date while two
/// upstream releases behind.
///
/// A strictly newer semver is enough on its own, even from a binary that has no
/// build number of its own (a crates.io tarball, a shallow clone). Within one
/// version the build number is still the only thing that can say "newer", so
/// there it keeps deciding — and with no number to compare, it stays quiet
/// rather than claiming an update forever.
fn is_newer_build(release: &ForkRelease) -> bool {
    let running = env!("CARGO_PKG_VERSION");
    if is_newer(&release.version, running) {
        return true;
    }
    if is_newer(running, &release.version) {
        return false;
    }
    current_build().is_some_and(|build| release.build > build)
}

/// `luvus update`: bring everything as up to date as it can be from inside a
/// running session, and be explicit about the one part that cannot.
///
/// Three tiers, cheapest first. The config, the installed themes and the agent
/// manifests are re-read live — no restart, no network. Git-installed modules
/// are pulled to their newest commit and swapped into the session, because they
/// are separate processes luvus only talks to. The core binary is the exception:
/// it is one statically linked executable with no dynamic loading, so a new one
/// can be put in place immediately but only takes effect at the next launch.
///
/// The fork publishes one bare binary per target and nothing else, so the
/// package-manager channels can only be reported, not driven: Homebrew and
/// crates.io serve upstream's luvus, which by definition never carries this
/// build.
pub fn run_cli(args: &[String]) -> Result<i32> {
    let yes = args.iter().any(|a| a == "--yes" || a == "-y");
    let restart = args.iter().any(|a| a == "--restart");
    if args
        .iter()
        .any(|a| !matches!(a.as_str(), "--yes" | "-y" | "--restart"))
    {
        eprintln!("usage: luvus update [--yes] [--restart]");
        return Ok(2);
    }

    // ── what a running session can swap without restarting ──
    match crate::cli::reload_running_session() {
        Ok(steps) => {
            println!("Reloaded in the running session:");
            for (label, outcome) in steps {
                println!("  {label:<16} {outcome}");
            }
        }
        Err(error) => println!("No running session to reload ({error})."),
    }

    // ── modules: separate processes, so a new version needs no restart ──
    let modules = crate::cli::update_modules(yes);
    if modules.is_empty() {
        println!("No modules installed.");
    } else {
        println!("Modules:");
        for (id, outcome) in modules {
            println!("  {id:<16} {outcome}");
        }
    }

    // ── the core binary, which does need one ──
    println!("Checking for Luvus updates...");
    let manifest = manifest_url();
    let current = current_build();
    let Some(release) = fetch_release(&manifest) else {
        bail!("could not check {manifest}; check your connection and try again")
    };
    let latest = release.label();
    if !is_newer_build(&release) {
        match current {
            Some(_) => println!("Luvus {} is already up to date.", installed_label()),
            None => println!(
                "This build has no fork build number, so {manifest} cannot be compared against it."
            ),
        }
        return Ok(0);
    }
    validate_release_version(&release.version)?;

    println!(
        "Luvus {latest} is available (current: {}).",
        installed_label()
    );
    let executable = std::env::current_exe().context("find the running Luvus binary")?;
    let executable = executable.canonicalize().unwrap_or(executable);
    let channel = classify_install(&executable, crate::platform::home_dir().as_deref());

    match channel {
        InstallChannel::Direct => install_direct_release(&release, &executable)?,
        InstallChannel::Development => bail!(
            "refusing to overwrite a development binary at {}; rebuild it with `cargo build --release`",
            executable.display()
        ),
        InstallChannel::Homebrew | InstallChannel::Cargo | InstallChannel::SystemPackage => bail!(
            "{} came from a package manager, which serves upstream Luvus and never this fork's builds; install the release binary from {RELEASE_BASE}/{} instead",
            executable.display(),
            release.tag
        ),
        InstallChannel::Nix => bail!(
            "this Luvus binary is managed by Nix; point your flake input at the fork's {} tag",
            release.tag
        ),
        InstallChannel::Unknown => bail!(
            "could not safely identify the installation channel for {}; update with the same method you originally installed Luvus",
            executable.display()
        ),
    }

    println!("Updated Luvus {} -> {latest}.", installed_label());
    if !restart {
        println!("Restart to load it: the ⟳ button in the changelog, or `luvus update --restart`.");
        return Ok(0);
    }

    // The restart tears down the server that owns the pane this command is
    // running in, so say what is about to happen before it happens.
    println!("Restarting — your session is saved and comes straight back.");
    match crate::cli::send_request("server.relaunch", serde_json::json!({})) {
        Ok(v) if v["result"]["type"] == "confirm_required" => {
            let message = v["result"]["message"]
                .as_str()
                .unwrap_or("agents are working");
            println!("{message} — run `luvus update --restart` again to restart anyway.");
        }
        Ok(v) if v.get("error").is_some() => {
            let message = v["error"]["message"].as_str().unwrap_or("failed");
            bail!("could not restart the running session: {message}")
        }
        Ok(_) => {}
        Err(_) => println!("No running session; the new binary loads on the next launch."),
    }
    Ok(0)
}

/// What this binary calls itself, build number and all.
fn installed_label() -> String {
    env!("LUVUS_VERSION_LABEL").to_string()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InstallChannel {
    Development,
    Homebrew,
    Cargo,
    Direct,
    Nix,
    SystemPackage,
    Unknown,
}

fn classify_install(executable: &Path, home: Option<&Path>) -> InstallChannel {
    let normalized = executable.to_string_lossy().replace('\\', "/");
    let lowered = normalized.to_ascii_lowercase();

    if lowered.contains("/target/debug/luvus") || lowered.contains("/target/release/luvus") {
        return InstallChannel::Development;
    }
    if lowered.contains("/cellar/luvus/") || lowered.contains("/homebrew/cellar/luvus/") {
        return InstallChannel::Homebrew;
    }
    if lowered.starts_with("/nix/store/") {
        return InstallChannel::Nix;
    }
    if matches!(lowered.as_str(), "/usr/bin/luvus" | "/bin/luvus") {
        return InstallChannel::SystemPackage;
    }

    if let Some(home) = home {
        let cargo = home.join(".cargo").join("bin").join(executable_name());
        if crate::platform::same_path(executable, &cargo) {
            return InstallChannel::Cargo;
        }
        let local = home.join(".local").join("bin").join(executable_name());
        if crate::platform::same_path(executable, &local) {
            return InstallChannel::Direct;
        }
    }

    if matches!(
        lowered.as_str(),
        "/usr/local/bin/luvus" | "/opt/local/bin/luvus"
    ) {
        return InstallChannel::Direct;
    }
    #[cfg(windows)]
    if lowered.ends_with("/luvus/luvus.exe") {
        return InstallChannel::Direct;
    }

    InstallChannel::Unknown
}

#[cfg(windows)]
fn executable_name() -> &'static str {
    "luvus.exe"
}

#[cfg(not(windows))]
fn executable_name() -> &'static str {
    "luvus"
}

fn validate_release_version(version: &str) -> Result<String> {
    let version = version.trim().trim_start_matches('v');
    semver::Version::parse(version)
        .map(|parsed| parsed.to_string())
        .map_err(|_| anyhow!("the update manifest returned an invalid version: {version:?}"))
}

fn verify_path_version(program: &Path, expected: &str) -> Result<()> {
    let output = crate::platform::no_window(&mut Command::new(program))
        .arg("--version")
        .output()
        .with_context(|| format!("verify updated binary `{}`", program.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || stdout.split_whitespace().nth(1) != Some(expected) {
        bail!(
            "the updater finished but `{}` reports {:?}, not Luvus {expected}",
            program.display(),
            stdout.trim()
        );
    }
    Ok(())
}

/// Fetch the release binary for this platform, check it against the published
/// digest, then swap it in. The fork publishes one bare executable per target
/// rather than an archive: nothing to extract, so nothing to get wrong, and it
/// works the same on a Windows box with no `tar` in reach.
fn install_direct_release(release: &ForkRelease, destination: &Path) -> Result<()> {
    let target = release_target()?;
    let name = format!("luvus-{target}{}", std::env::consts::EXE_SUFFIX);
    let base = std::env::var("LUVUS_UPDATE_RELEASE_BASE")
        .unwrap_or_else(|_| format!("{RELEASE_BASE}/{}", release.tag));
    let base = base.trim_end_matches('/');
    let temp = UpdateTempDir::new()?;
    let binary = temp.path().join(&name);
    let checksum = temp.path().join(format!("{name}.sha256"));

    download_file(&format!("{base}/{name}"), &binary)?;
    download_file(&format!("{base}/{name}.sha256"), &checksum)?;
    verify_sha256(&binary, &checksum)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))
            .context("make the downloaded binary executable")?;
    }
    verify_path_version(&binary, &release.version)?;
    replace_executable(&binary, destination)?;
    verify_path_version(destination, &release.version)
}

fn release_target() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-musl"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-musl"),
        ("windows", "aarch64") => Ok("aarch64-pc-windows-msvc"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        (os, arch) => bail!("no prebuilt Luvus release exists for {os}/{arch}"),
    }
}

fn download_file(url: &str, destination: &Path) -> Result<()> {
    let curl = crate::platform::no_window(&mut Command::new("curl"))
        .args(["-fsSL", "--max-time", "120", "-H", "User-Agent: luvus"])
        .arg("-o")
        .arg(destination)
        .arg(url)
        .status();
    if matches!(curl, Ok(status) if status.success()) {
        return Ok(());
    }

    let wget = crate::platform::no_window(&mut Command::new("wget"))
        .args(["-q", "--timeout=120", "--header=User-Agent: luvus"])
        .arg("-O")
        .arg(destination)
        .arg(url)
        .status();
    if matches!(wget, Ok(status) if status.success()) {
        return Ok(());
    }
    bail!("download failed: {url} (install curl or wget, then try again)")
}

fn verify_sha256(archive: &Path, checksum_file: &Path) -> Result<()> {
    let expected_body = fs::read_to_string(checksum_file)
        .with_context(|| format!("read checksum {}", checksum_file.display()))?;
    let expected = expected_body.split_whitespace().next().unwrap_or("");
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("release checksum is not a valid SHA-256 digest");
    }

    let mut file = fs::File::open(archive)
        .with_context(|| format!("open downloaded archive {}", archive.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        bail!("release checksum mismatch; the existing Luvus binary was not changed");
    }
    Ok(())
}

/// Windows refuses to *overwrite* a running image but happily **renames** it,
/// which is how every Windows updater does this: move the old binary aside, drop
/// the new one in its place, and let the already-running process keep reading
/// the renamed file through its open handle until it exits. The update is on
/// disk immediately; it takes effect the next time luvus starts.
#[cfg(windows)]
fn replace_executable(candidate: &Path, destination: &Path) -> Result<()> {
    let retired = retire_path(destination);
    if destination.exists() {
        fs::rename(destination, &retired)
            .with_context(|| format!("move the running {} aside", destination.display()))?;
    }
    if let Err(error) = fs::copy(candidate, destination) {
        // Put the old binary back rather than leaving no luvus at all.
        let _ = fs::rename(&retired, destination);
        return Err(error).with_context(|| format!("install the new {}", destination.display()));
    }
    // Only now, with the new binary in place, sweep what *earlier* updates left
    // behind — never the one just made, which is the rollback. Whatever is still
    // running keeps its file; the rest go.
    sweep_retired(destination, &retired);
    Ok(())
}

/// A free name to move the outgoing binary to, next to it.
///
/// Never a fixed `luvus.old.exe`. Windows lets you rename a running image but
/// not delete one, and `rename` onto an existing path has to delete what is
/// there — so the moment any earlier build was still running under that one
/// name, every future update failed at this step. That is not a rare corner:
/// this fork's restart hands the console over by leaving the old process in
/// `wait()`, so a previous build running is the normal state after an update.
///
/// Ten candidates is plenty — they only survive while their process does.
#[cfg(windows)]
fn retire_path(destination: &Path) -> PathBuf {
    for n in 0..10 {
        let candidate = destination.with_extension(format!("old{n}.exe"));
        if !candidate.exists() {
            return candidate;
        }
    }
    // Everything taken means ten live builds, which cannot really happen; fall
    // back to a stamped name rather than returning a path that must fail.
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    destination.with_extension(format!("old-{stamp}.exe"))
}

/// Delete the retired binaries of past updates, leaving `keep` — the rollback
/// this update just made — alone. Each is held open for as long as the build it
/// belongs to is still running, so a failure here means exactly "that one is
/// still in use" and is not worth reporting.
#[cfg(windows)]
fn sweep_retired(destination: &Path, keep: &Path) {
    let Some(dir) = destination.parent() else {
        return;
    };
    let Some(stem) = destination.file_stem().and_then(|s| s.to_str()) else {
        return;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        // `luvus.old3.exe`, `luvus.old-1756223.exe`, and the `luvus.old.exe` of
        // every build that predates this scheme.
        if name.starts_with(&format!("{stem}.old"))
            && name.ends_with(".exe")
            && !crate::platform::same_path(&entry.path(), keep)
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(not(windows))]
fn replace_executable(candidate: &Path, destination: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("installed binary has no parent directory"))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let staging = parent.join(format!(".luvus-update-{}-{nonce}", std::process::id()));

    match fs::copy(candidate, &staging) {
        Ok(_) => {
            fs::set_permissions(&staging, fs::Permissions::from_mode(0o755))?;
            if let Err(error) = fs::rename(&staging, destination) {
                let _ = fs::remove_file(&staging);
                return Err(error)
                    .with_context(|| format!("atomically replace {}", destination.display()));
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            let stage_status = Command::new("sudo")
                .args(["install", "-m", "0755"])
                .arg(candidate)
                .arg(&staging)
                .status()
                .with_context(|| {
                    format!(
                        "stage an update beside {} with administrator permission",
                        destination.display(),
                    )
                })?;
            if !stage_status.success() {
                bail!(
                    "could not stage the update beside {} with sudo ({stage_status})",
                    destination.display(),
                );
            }

            let replace_status = Command::new("sudo")
                .arg("mv")
                .arg("-f")
                .arg(&staging)
                .arg(destination)
                .status()
                .with_context(|| {
                    format!(
                        "atomically replace {} with administrator permission",
                        destination.display(),
                    )
                })?;
            if !replace_status.success() {
                let _ = Command::new("sudo")
                    .args(["rm", "-f"])
                    .arg(&staging)
                    .status();
                bail!(
                    "could not replace {} with sudo ({replace_status})",
                    destination.display(),
                );
            }
            Ok(())
        }
        Err(error) => {
            Err(error).with_context(|| format!("stage update beside {}", destination.display()))
        }
    }
}

struct UpdateTempDir(PathBuf);

impl UpdateTempDir {
    fn new() -> Result<Self> {
        let base = std::env::temp_dir();
        for attempt in 0..32_u8 {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = base.join(format!(
                "luvus-update-{}-{nonce}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    // Windows already hands each user a private `%TEMP%`.
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if let Err(error) =
                            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                        {
                            let _ = fs::remove_dir(&path);
                            return Err(error).context("make the update directory private");
                        }
                    }
                    return Ok(Self(path));
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error).context("create a private update directory"),
            }
        }
        bail!("could not create a unique update directory")
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for UpdateTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Check now and report the outcome, whatever it is (the explicit button).
pub fn check_now_reporting(tx: Sender<AppEvent>) {
    thread::spawn(move || {
        let _ = tx.send(AppEvent::UpdateChecked(fetch_outcome(&manifest_url())));
    });
}

/// One fetch-compare-report, silent unless there is news. Takes the URL so tests
/// can point it at a file without mutating process-wide environment.
///
/// With `auto_install` the same pass also *applies* the update, but only to an
/// installation luvus itself owns (see [`install_in_place`]). A binary managed
/// by Homebrew, Cargo or an OS package is left alone: replacing it behind the
/// package manager's back is how you end up with an installation neither side
/// can reason about.
fn check_once(tx: &Sender<AppEvent>, url: &str, auto_install: bool) {
    let Some(release) = fetch_release(url).filter(is_newer_build) else {
        return;
    };
    let _ = tx.send(AppEvent::UpdateAvailable(release.label()));
    if !auto_install {
        return;
    }
    // Say so when it fails. This used to be `.unwrap_or(false)`, which turned
    // every install error into silence: the update was announced, the install
    // died on something as ordinary as an old binary that could not be moved
    // aside, and nothing distinguished that from having nothing to install.
    match install_in_place(&release) {
        Ok(true) => {
            let _ = tx.send(AppEvent::SelfUpdateInstalled(release.label()));
        }
        // Not a direct install — a package manager owns this binary, and saying
        // so on every check would be nagging about a thing luvus must not touch.
        Ok(false) => {}
        Err(error) => {
            let _ = tx.send(AppEvent::SelfUpdateFailed(format!("{error:#}")));
        }
    }
}

/// Tell the app when upstream cut a release newer than the one this fork was
/// merged from. Informational only — there is no button, because catching up
/// means merging `upstream/main` by hand.
fn check_upstream_once(tx: &Sender<AppEvent>, url: &str) {
    if let Some(tag) = http_get(url).as_deref().and_then(parse_tag_name) {
        if is_newer(&tag, CURRENT) {
            let _ = tx.send(AppEvent::UpstreamUpdateAvailable(tag));
        }
    }
}

/// Download and swap in a newer fork build, in the background, for a direct
/// install only. `Ok(false)` means "not ours to touch", which is not an error.
fn install_in_place(release: &ForkRelease) -> Result<bool> {
    let executable = std::env::current_exe().context("find the running Luvus binary")?;
    let executable = executable.canonicalize().unwrap_or(executable);
    if classify_install(&executable, crate::platform::home_dir().as_deref())
        != InstallChannel::Direct
    {
        return Ok(false);
    }
    install_direct_release(release, &executable)?;
    Ok(true)
}

/// Read the fork manifest: `{"version": "0.12.0", "build": 49, "tag": "build-49"}`.
/// `tag` is optional and defaults to `build-<n>`, which is what the release
/// workflow names them.
fn parse_manifest(body: &str) -> Option<ForkRelease> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let version = v.get("version")?.as_str()?.trim().trim_start_matches('v');
    let build = u32::try_from(v.get("build")?.as_u64()?).ok()?;
    let tag = v
        .get("tag")
        .and_then(|t| t.as_str())
        .map(|t| t.trim().to_string())
        .unwrap_or_else(|| format!("build-{build}"));
    Some(ForkRelease {
        version: version.to_string(),
        build,
        tag,
    })
}

/// Pull `tag_name` out of a GitHub `releases/latest` response.
fn parse_tag_name(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let tag = v.get("tag_name")?.as_str()?.trim();
    Some(tag.trim_start_matches('v').to_string())
}

/// True when `latest` is a strictly higher semver than `current`. Both accept an
/// optional leading `v`; any pre-release/build suffix on a component is ignored.
pub fn is_newer(latest: &str, current: &str) -> bool {
    semver(latest) > semver(current)
}

fn semver(s: &str) -> (u32, u32, u32) {
    let s = s.trim().trim_start_matches('v');
    let mut it = s.split('.').map(|part| {
        part.split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap_or("")
            .parse::<u32>()
            .unwrap_or(0)
    });
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

/// Fetch a URL with `curl`, then `wget` — whichever is installed. `None` on any
/// failure (offline, tool missing, non-200): a missed check is a silent no-op.
fn http_get(url: &str) -> Option<String> {
    let curl = ["-fsSL", "--max-time", "15", "-H", "User-Agent: luvus", url];
    if let Some(out) = try_cmd("curl", &curl) {
        return Some(out);
    }
    let wget = [
        "-q",
        "-O",
        "-",
        "--timeout=15",
        "--header=User-Agent: luvus",
        url,
    ];
    try_cmd("wget", &wget)
}

fn try_cmd(prog: &str, args: &[&str]) -> Option<String> {
    let out = crate::platform::no_window(Command::new(prog).args(args))
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_compares_semver_with_optional_v() {
        assert!(is_newer("0.9.3", "0.9.2"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.9.2", "0.9.2"), "same version is not newer");
        assert!(!is_newer("0.9.1", "0.9.2"), "older is not newer");
        // A pre-release suffix on a component doesn't break the compare.
        assert!(is_newer("0.9.3-rc1", "0.9.2"));
    }

    /// The build number restarts from zero at every upstream release the fork
    /// merges, so across a version bump the newer release carries the *smaller*
    /// number. Deciding on that number alone stranded the updater: it refused
    /// the new release, and every release after it, while reporting no update
    /// available.
    #[test]
    fn a_newer_release_wins_even_though_its_build_number_is_lower() {
        let running = env!("CARGO_PKG_VERSION");
        let release = |version: &str, build: u32| ForkRelease {
            version: version.to_string(),
            build,
            tag: format!("build-{build}"),
        };

        // The exact shape that stranded it: running 0.12.0 at build 94, the
        // first build of 0.13.1 counted only 68 commits past its own tag.
        assert!(
            is_newer_build(&release("99.0.0", 0)),
            "a newer release is newer whatever its build number"
        );
        assert!(
            !is_newer_build(&release("0.0.1", u32::MAX)),
            "an older release never wins on build number alone"
        );

        // Within one version the build number still decides, as it always did:
        // every fork release carries upstream's semver, so nothing else can.
        let Some(build) = super::current_build() else {
            return; // built without the release tag in reach; nothing to compare
        };
        assert!(is_newer_build(&release(running, build + 1)));
        assert!(
            !is_newer_build(&release(running, build)),
            "same is not newer"
        );
        assert!(!is_newer_build(&release(running, build.saturating_sub(1))));
    }

    /// The whole chain, off the network: fetch, parse, compare, report. A file
    /// URL rather than the env override, so this cannot race another test.
    #[test]
    fn check_once_reports_only_a_higher_build() {
        use std::sync::mpsc::channel;
        let Some(build) = super::current_build() else {
            return; // built without the release tag in reach; nothing to compare
        };
        let dir = std::env::temp_dir().join(format!("luvus-upd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("latest.json");
        let url = format!("file://{}", path.display());
        let manifest = |b: u32| {
            format!(
                r#"{{"version":"{}","build":{b},"tag":"build-{b}"}}"#,
                super::CURRENT
            )
        };

        // A higher build is news, and it is announced with the label the
        // sidebar uses, not the bare semver both sides share.
        std::fs::write(&path, manifest(build + 1)).unwrap();
        let (tx, rx) = channel();
        super::check_once(&tx, &url, false);
        match rx.try_recv() {
            Ok(crate::event::AppEvent::UpdateAvailable(v)) => {
                assert_eq!(v, format!("{} - 0.{:02}", super::CURRENT, build + 1));
            }
            _ => panic!("a higher build should have been reported"),
        }

        // This build, and an older one: silence. Upstream's semver is identical
        // in all three, which is exactly why the build number has to decide.
        for b in [build, build.saturating_sub(1)] {
            std::fs::write(&path, manifest(b)).unwrap();
            let (tx, rx) = channel();
            super::check_once(&tx, &url, false);
            assert!(rx.try_recv().is_err(), "build {b} must not be reported");
        }

        // Unreachable manifest, and junk: no panic, no event.
        for bad in [
            format!("file://{}", dir.join("nope.json").display()),
            url.clone(),
        ] {
            if bad == url {
                std::fs::write(&path, "not json").unwrap();
            }
            let (tx, rx) = channel();
            super::check_once(&tx, &bad, false);
            assert!(rx.try_recv().is_err());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_the_fork_manifest() {
        let release = parse_manifest(r#"{"version":"v0.12.0","build":49,"tag":"build-49"}"#)
            .expect("a well-formed manifest");
        assert_eq!(release.version, "0.12.0"); // leading `v` trimmed
        assert_eq!(release.build, 49);
        assert_eq!(release.tag, "build-49");
        assert_eq!(release.label(), "0.12.0 - 0.49");

        // `tag` is optional: the workflow names releases after the build.
        let implied = parse_manifest(r#"{"version":"0.12.0","build":7}"#).expect("no tag");
        assert_eq!(implied.tag, "build-7");
        assert_eq!(implied.label(), "0.12.0 - 0.07");

        // Garbage, or a manifest with no build number → None. Upstream's own
        // `luvus.dev/latest.json` lands here, and must not read as an update.
        assert!(parse_manifest("not json").is_none());
        assert!(parse_manifest(r#"{"version":"0.13.0"}"#).is_none());
        assert!(parse_manifest(r#"{"build":49}"#).is_none());
    }

    /// The upstream probe reads GitHub's release JSON and compares semver — the
    /// one place where semver still decides anything.
    #[test]
    fn upstream_probe_reads_the_release_tag() {
        assert_eq!(
            parse_tag_name(r#"{"tag_name":"v0.13.0","name":"0.13.0"}"#).as_deref(),
            Some("0.13.0")
        );
        assert!(parse_tag_name(r#"{"message":"Not Found"}"#).is_none());
        assert!(is_newer("0.13.0", super::CURRENT));
        assert!(!is_newer(super::CURRENT, super::CURRENT));
    }

    /// Every `luvus.old*.exe` in `dir`, sorted — the rollbacks an install left.
    #[cfg(windows)]
    fn retired_files(dir: &std::path::Path) -> Vec<String> {
        let mut found: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.starts_with("luvus.old"))
            .collect();
        found.sort();
        found
    }

    /// Windows cannot overwrite a running image, so the installer renames the
    /// old binary aside and drops the new one into the freed path. The old one
    /// has to survive — the running process is still reading it, and it is the
    /// only rollback there is — while earlier ones are cleared rather than piling up.
    #[cfg(windows)]
    #[test]
    fn windows_install_moves_the_running_binary_aside() {
        let dir = std::env::temp_dir().join(format!("luvus-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let destination = dir.join("luvus.exe");
        let candidate = dir.join("new.exe");
        std::fs::write(&destination, b"old").unwrap();
        std::fs::write(&candidate, b"new").unwrap();

        super::replace_executable(&candidate, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"new");
        let retired = retired_files(&dir);
        assert_eq!(retired.len(), 1, "exactly one rollback: {retired:?}");
        assert_eq!(
            std::fs::read(dir.join(&retired[0])).unwrap(),
            b"old",
            "the replaced binary is kept as the rollback"
        );

        // A second update clears the previous rollback rather than piling up.
        std::fs::write(&candidate, b"newer").unwrap();
        super::replace_executable(&candidate, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"newer");
        let retired = retired_files(&dir);
        assert_eq!(retired.len(), 1, "still exactly one: {retired:?}");
        assert_eq!(std::fs::read(dir.join(&retired[0])).unwrap(), b"new");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The bug that stranded a real install. A rollback whose build is still
    /// running cannot be deleted, and `rename` onto an existing path has to
    /// delete what is there — so reusing one fixed `luvus.old.exe` made every
    /// later update fail at that step, and the error was swallowed, so the
    /// update button did nothing and said nothing.
    ///
    /// The lock is the real thing, not a stand-in: a handle opened without
    /// `FILE_SHARE_DELETE`, which is how Windows holds a running image and the
    /// only property that mattered. Read-only would not do — `remove_file`
    /// clears that attribute itself and the file would vanish.
    #[cfg(windows)]
    #[test]
    fn an_undeletable_rollback_does_not_block_the_next_update() {
        let dir = std::env::temp_dir().join(format!("luvus-locked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let destination = dir.join("luvus.exe");
        let candidate = dir.join("new.exe");
        std::fs::write(&destination, b"old").unwrap();
        std::fs::write(&candidate, b"new").unwrap();

        // What every build before this scheme left behind, still in use.
        let stuck = dir.join("luvus.old.exe");
        std::fs::write(&stuck, b"ancient").unwrap();
        let handle = deny_delete(&stuck);
        assert!(
            std::fs::remove_file(&stuck).is_err(),
            "the setup is only meaningful if the file really cannot be deleted"
        );

        super::replace_executable(&candidate, &destination)
            .expect("an undeletable rollback must not stop the install");
        assert_eq!(std::fs::read(&destination).unwrap(), b"new");
        assert_eq!(
            std::fs::read(&stuck).unwrap(),
            b"ancient",
            "the one still in use is left exactly as it was"
        );

        unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Hold `path` open the way Windows holds a running executable: readable by
    /// others, but not deletable. Close the returned handle when done.
    #[cfg(windows)]
    fn deny_delete(path: &std::path::Path) -> windows_sys::Win32::Foundation::HANDLE {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, OPEN_EXISTING,
        };

        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                windows_sys::Win32::Foundation::GENERIC_READ,
                FILE_SHARE_READ, // no FILE_SHARE_DELETE: this is the whole point
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };
        assert!(
            !handle.is_null() && handle != windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
            "could not open {} to hold it",
            path.display()
        );
        handle
    }

    #[test]
    fn validates_versions_before_using_them_in_release_urls() {
        assert_eq!(validate_release_version("v1.2.3").unwrap(), "1.2.3");
        assert!(validate_release_version("1.2.3/../../asset").is_err());
        assert!(validate_release_version("latest").is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn classifies_supported_and_managed_install_paths() {
        let home = Path::new("/home/alice");
        assert_eq!(
            classify_install(Path::new("/work/luvus/target/debug/luvus"), Some(home)),
            InstallChannel::Development
        );
        assert_eq!(
            classify_install(
                Path::new("/opt/homebrew/Cellar/luvus/0.12.0/bin/luvus"),
                Some(home)
            ),
            InstallChannel::Homebrew
        );
        assert_eq!(
            classify_install(Path::new("/home/alice/.cargo/bin/luvus"), Some(home)),
            InstallChannel::Cargo
        );
        assert_eq!(
            classify_install(Path::new("/home/alice/.local/bin/luvus"), Some(home)),
            InstallChannel::Direct
        );
        assert_eq!(
            classify_install(Path::new("/nix/store/hash-luvus/bin/luvus"), Some(home)),
            InstallChannel::Nix
        );
        assert_eq!(
            classify_install(Path::new("/usr/bin/luvus"), Some(home)),
            InstallChannel::SystemPackage
        );
        assert_eq!(
            classify_install(Path::new("/opt/mise/bin/luvus"), Some(home)),
            InstallChannel::Unknown
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn verifies_release_sha256_before_installing() {
        let temp = UpdateTempDir::new().unwrap();
        let archive = temp.path().join("release.tar.gz");
        let checksum = temp.path().join("release.sha256");
        fs::write(&archive, b"abc").unwrap();
        fs::write(
            &checksum,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  release.tar.gz\n",
        )
        .unwrap();
        verify_sha256(&archive, &checksum).unwrap();

        fs::write(&archive, b"changed").unwrap();
        assert!(verify_sha256(&archive, &checksum)
            .unwrap_err()
            .to_string()
            .contains("checksum mismatch"));
    }
}
