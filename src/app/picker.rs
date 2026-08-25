//! The folder picker — a modal to open (or create) a folder as a new **static
//! workspace** (workspace). The "+" button opens it: browse the filesystem, pick an
//! existing folder, or make a new one (which opens immediately). When the browsed
//! folder is a git repo it offers a second action row, **"Open with new
//! worktree"** (`w` also triggers it). The front door for workspaces and worktrees.

use std::path::PathBuf;

use super::*;

/// One entry in the browsed directory — a subfolder (navigable) or a file
/// (shown so you can see the folder has content, but not selectable).
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}

/// State of the open folder picker (workspace chooser).
pub struct FolderPicker {
    /// The directory currently being browsed.
    pub path: PathBuf,
    /// Folders + files in `path`, dirs first then files (dotfiles unless
    /// [`FolderPicker::show_hidden`]).
    pub entries: Vec<Entry>,
    /// Cursor into the row list (see [`Row`] / [`FolderPicker::row`]).
    pub cursor: usize,
    /// When making a new folder, the name being typed.
    pub creating: Option<String>,
    /// macOS-style "Go to" input. Enter navigates to this path but deliberately
    /// does not open it as a workspace; the OpenFolder row remains confirmation.
    pub going_to: Option<String>,
    /// Last filesystem error (e.g. permission denied), shown in the modal.
    pub error: Option<String>,
    /// Whether the browsed folder is a git repo — adds the "Open with new
    /// worktree" row (and the `w` accelerator). Recomputed when the path changes.
    pub is_repo: bool,
    /// Whether dotfile entries are listed (`.` toggles).
    pub show_hidden: bool,
}

/// A selectable row in the picker. The action rows lead; the directory entries
/// follow. The "open with worktree" row only exists when the folder is a repo.
#[derive(Debug)]
pub enum Row {
    /// Open the browsed folder as a workspace.
    OpenFolder,
    /// Create a git worktree of the browsed repo (then open it).
    OpenWorktree,
    /// Jump to the user's home directory without opening it.
    Home,
    /// `..` — go to the parent directory.
    Up,
    /// `entries[idx]`.
    Entry(usize),
}

/// Mouse targets rendered by the picker. Modal is last in hit-test order so
/// rows and the footer hints remain interactive while inert modal space simply
/// keeps the picker open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerHit {
    Row(usize),
    /// A footer key hint; a click behaves exactly like pressing that key.
    Hint(KeyCode),
    Modal,
}

impl FolderPicker {
    /// Is the picker on the drive list — the virtual folder above `C:\` that
    /// [`crate::platform::drive_roots`] fills? Marked by an empty path, which is
    /// never a real browsed folder.
    pub fn at_drives(&self) -> bool {
        self.path.as_os_str().is_empty()
    }

    /// Number of action rows before the directory entries: "open" + (optional)
    /// "open with worktree" + "home" + "..".
    fn leading(&self) -> usize {
        if self.at_drives() {
            // The drive list is not a folder to open, and nothing is above it.
            // `~` and `g` still work as accelerators; they just have no row here.
            0
        } else if self.is_repo {
            4
        } else {
            3
        }
    }

    /// Total selectable rows.
    pub fn row_count(&self) -> usize {
        self.leading() + self.entries.len()
    }

    /// Classify the row at index `i`.
    pub fn row(&self, i: usize) -> Row {
        let leading = self.leading();
        match (i, self.is_repo) {
            // Checked first so a drive list, whose `leading` is 0, is all entries
            // rather than falling into the action arms below.
            _ if i >= leading => Row::Entry(i - leading),
            (0, _) => Row::OpenFolder,
            (1, true) => Row::OpenWorktree,
            (1, false) | (2, true) => Row::Home,
            _ => Row::Up,
        }
    }
}

impl App {
    /// Open the folder picker, starting in the active workspace's folder (or `$HOME`).
    pub fn open_folder_picker(&mut self) {
        let start = self
            .workspaces
            .get(self.active_ws)
            .map(|w| w.cwd.clone())
            .filter(|p| p.is_dir())
            .or_else(crate::platform::home_dir)
            .unwrap_or_else(|| PathBuf::from("/"));
        self.open_folder_picker_at(start);
    }

    /// Open the folder picker starting at `start` (falls back to `$HOME` if it's
    /// not a directory). Used by the workspace menu's "Open worktree".
    pub fn open_folder_picker_at(&mut self, start: PathBuf) {
        let start = start
            .is_dir()
            .then_some(start)
            .or_else(crate::platform::home_dir)
            .unwrap_or_else(|| PathBuf::from("/"));
        self.picker = Some(FolderPicker {
            path: start,
            entries: Vec::new(),
            cursor: 0,
            creating: None,
            going_to: None,
            error: None,
            is_repo: false,
            show_hidden: false,
        });
        self.picker_refresh();
    }

    pub fn close_folder_picker(&mut self) {
        self.picker = None;
    }

    /// Re-read the browsed path's entries (folders + files), dirs first. On the
    /// drive list there is nothing to read: the entries *are* the drives.
    fn picker_refresh(&mut self) {
        // Remember which entry the cursor highlights so filter changes (e.g.
        // `.` hiding dotfiles) re-anchor the selection by identity instead of
        // leaving it at a numeric index that may now point elsewhere.
        let selected = self.picker.as_ref().and_then(|p| match p.row(p.cursor) {
            Row::Entry(idx) => p.entries.get(idx).map(|e| e.name.clone()),
            _ => None,
        });
        if let Some(p) = self.picker.as_mut() {
            if p.at_drives() {
                p.entries = crate::platform::drive_roots()
                    .into_iter()
                    .map(|d| Entry {
                        name: d.display().to_string(),
                        is_dir: true,
                    })
                    .collect();
                p.is_repo = false;
                p.cursor = p.cursor.min(p.row_count().saturating_sub(1));
                return;
            }
            let mut entries: Vec<Entry> = std::fs::read_dir(&p.path)
                .map(|rd| {
                    rd.filter_map(Result::ok)
                        .filter_map(|e| {
                            let name = e.file_name().into_string().ok()?;
                            if !p.show_hidden && name.starts_with('.') {
                                return None;
                            }
                            let is_dir = e.file_type().map(|ty| ty.is_dir()).unwrap_or(false);
                            Some(Entry { name, is_dir })
                        })
                        .collect()
                })
                .unwrap_or_default();
            // Folders first, then files; each alphabetical (case-insensitive).
            entries.sort_by(|a, b| {
                b.is_dir
                    .cmp(&a.is_dir)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            p.entries = entries;
            if let Some(name) = selected {
                match p.entries.iter().position(|e| e.name == name) {
                    Some(pos) => p.cursor = p.leading() + pos,
                    // Highlighted entry was filtered out: fall back to the
                    // last fixed action row instead of letting the stale
                    // index land on some other directory.
                    None => p.cursor = p.leading() - 1,
                }
            }
            p.is_repo = crate::git::local::is_repo(&p.path);
            p.cursor = p.cursor.min(p.row_count().saturating_sub(1));
        }
    }

    /// Show/hide dotfile entries (`.` or the footer hint).
    pub fn picker_toggle_hidden(&mut self) {
        if let Some(p) = self.picker.as_mut() {
            p.show_hidden = !p.show_hidden;
        }
        self.picker_refresh();
    }

    /// The "Open with new worktree" row (or `w`): create a git worktree of the
    /// browsed repo. Hands off to the branch prompt (targeting this folder), so
    /// the flow matches `Ctrl+Space G`.
    fn picker_make_worktree(&mut self) {
        let repo = self
            .picker
            .as_ref()
            .filter(|p| p.is_repo)
            .map(|p| p.path.clone());
        if let Some(repo) = repo {
            self.picker = None;
            self.worktree_repo = Some(repo);
            self.worktree_prompt = Some(String::new());
        }
    }

    /// Consume a paste without letting it reach the pane behind the picker.
    /// Text sub-modes receive text; otherwise a pasted path navigates directly.
    pub fn picker_paste(&mut self, raw: &str) {
        let text: String = raw.chars().filter(|c| !c.is_control()).collect();
        if text.is_empty() {
            return;
        }
        if let Some(picker) = self.picker.as_mut() {
            if let Some(buffer) = picker.creating.as_mut() {
                buffer.push_str(&text);
                picker.error = None;
                return;
            }
            if let Some(buffer) = picker.going_to.as_mut() {
                buffer.push_str(&text);
                picker.error = None;
                return;
            }
        }
        self.picker_go_to(text);
    }

    /// Key handling while the folder picker is open.
    pub fn handle_picker_key(&mut self, key: KeyEvent) {
        // New-folder name input sub-mode.
        if let Some(p) = self.picker.as_mut() {
            if let Some(buf) = p.creating.as_mut() {
                match key.code {
                    KeyCode::Esc => {
                        p.creating = None;
                        p.error = None;
                    }
                    KeyCode::Enter => {
                        let name = buf.clone();
                        self.picker_create_folder(name);
                    }
                    KeyCode::Backspace => {
                        buf.pop();
                    }
                    KeyCode::Char(c) => buf.push(c),
                    _ => {}
                }
                return;
            }
            if let Some(buf) = p.going_to.as_mut() {
                match key.code {
                    KeyCode::Esc => {
                        p.going_to = None;
                        p.error = None;
                    }
                    KeyCode::Enter => {
                        let path = buf.clone();
                        self.picker_go_to(path);
                    }
                    KeyCode::Backspace => {
                        buf.pop();
                        p.error = None;
                    }
                    KeyCode::Char(c) => {
                        buf.push(c);
                        p.error = None;
                    }
                    _ => {}
                }
                return;
            }
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.picker_move(1),
            KeyCode::Char('k') | KeyCode::Up => self.picker_move(-1),
            KeyCode::Left | KeyCode::Backspace | KeyCode::Char('h') => self.picker_up(),
            KeyCode::Right | KeyCode::Char('l') => self.picker_descend(),
            KeyCode::Enter => self.picker_activate(),
            KeyCode::Char('n') => {
                // Not on the drive list: there is no folder to create in, and the
                // relative `create_dir` would land in luvus' own working directory.
                if let Some(p) = self.picker.as_mut().filter(|p| !p.at_drives()) {
                    p.creating = Some(String::new());
                    p.going_to = None;
                    p.error = None;
                }
            }
            KeyCode::Char('g') => self.picker_start_go_to(),
            KeyCode::Char('.') => self.picker_toggle_hidden(),
            KeyCode::Home | KeyCode::Char('~') => self.picker_home(),
            KeyCode::Char('w') => self.picker_make_worktree(),
            KeyCode::Esc | KeyCode::Char('q') => self.close_folder_picker(),
            _ => {}
        }
    }

    fn picker_move(&mut self, delta: i32) {
        if let Some(p) = self.picker.as_mut() {
            let max = p.row_count().saturating_sub(1) as i32;
            p.cursor = (p.cursor as i32 + delta).clamp(0, max) as usize;
        }
    }

    /// Wheel-scroll the browse list by `delta` rows (cursor stays in view).
    pub fn picker_scroll(&mut self, delta: i32) {
        self.picker_move(delta);
    }

    /// Browse up to the parent directory — and, above a Windows drive root, to
    /// the list of drives. `C:\` has no parent, so without that step `..` was a
    /// dead end and the only way to another drive was knowing its letter and
    /// typing it into "Go to".
    fn picker_up(&mut self) {
        if let Some(p) = self.picker.as_mut() {
            if let Some(parent) = p.path.parent().map(PathBuf::from) {
                p.path = parent;
                p.cursor = 0;
            } else if cfg!(windows) && !p.at_drives() {
                p.path = PathBuf::new();
                p.cursor = 0;
            }
        }
        self.picker_refresh();
    }

    /// Browse the home directory without opening a workspace.
    fn picker_home(&mut self) {
        let Some(home) = crate::platform::home_dir().filter(|path| path.is_dir()) else {
            let error = self.catalog.home_unavailable.to_string();
            if let Some(p) = self.picker.as_mut() {
                p.error = Some(error);
            }
            return;
        };
        if let Some(p) = self.picker.as_mut() {
            p.path = home;
            p.cursor = 0;
            p.error = None;
        }
        self.picker_refresh();
    }

    /// Start the in-modal path navigator. It is intentionally separate from
    /// opening a workspace so Enter cannot accidentally confirm a folder.
    pub fn picker_start_go_to(&mut self) {
        if let Some(p) = self.picker.as_mut() {
            p.creating = None;
            p.going_to = Some(String::new());
            p.error = None;
        }
    }

    /// Resolve an entered path and browse to it. Absolute paths, paths relative
    /// to the currently browsed folder, `~` / `~/...`, and a bare drive letter
    /// (`D:`, the way a drive change is typed) are supported. A file path
    /// browses its parent directory without opening a workspace.
    fn picker_go_to(&mut self, input: String) {
        let entered = input.trim();
        let entered = entered
            .strip_prefix('"')
            .and_then(|text| text.strip_suffix('"'))
            .or_else(|| {
                entered
                    .strip_prefix('\'')
                    .and_then(|text| text.strip_suffix('\''))
            })
            .unwrap_or(entered)
            .trim();
        if entered.is_empty() {
            let error = self.catalog.enter_folder_path.to_string();
            if let Some(p) = self.picker.as_mut() {
                p.error = Some(error);
            }
            return;
        }

        let current = self.picker.as_ref().map(|p| p.path.clone());
        // `~`, `~/…` and a bare drive letter all resolve here, in the one place
        // that knows what a path typed by a person means.
        let path = crate::platform::user_path(entered);
        let path = if path.is_absolute() {
            path
        } else {
            current.unwrap_or_default().join(path)
        };

        // A *file* lands on its folder: dragging one in is a normal way to say
        // "this project", and it must not leave the picker on a path that is
        // not a directory.
        let target = if path.is_dir() {
            Some(path)
        } else if path.is_file() {
            path.parent().map(PathBuf::from)
        } else {
            None
        };
        let Some(target) = target else {
            let error = format!("{}: {entered}", self.catalog.folder_not_found);
            if let Some(p) = self.picker.as_mut() {
                p.error = Some(error);
            }
            return;
        };

        if let Some(p) = self.picker.as_mut() {
            p.path = target;
            p.cursor = 0;
            p.going_to = None;
            p.error = None;
        }
        self.picker_refresh();
    }

    /// Browse into the highlighted subdirectory (only folder entries navigate).
    fn picker_descend(&mut self) {
        let target = self.picker.as_ref().and_then(|p| match p.row(p.cursor) {
            Row::Entry(idx) => p
                .entries
                .get(idx)
                .filter(|e| e.is_dir)
                .map(|e| p.path.join(&e.name)),
            _ => None,
        });
        if let Some(t) = target {
            if let Some(p) = self.picker.as_mut() {
                p.path = t;
                p.cursor = 0;
            }
            self.picker_refresh();
        }
    }

    /// `⏎` / click — contextual on the highlighted row.
    pub fn picker_activate(&mut self) {
        let Some(row) = self.picker.as_ref().map(|p| p.row(p.cursor)) else {
            return;
        };
        match row {
            // Open the current folder as a new static workspace.
            Row::OpenFolder => {
                if let Some(p) = self.picker.take() {
                    self.open_workspace_at(p.path);
                }
            }
            Row::OpenWorktree => self.picker_make_worktree(),
            Row::Home => self.picker_home(),
            Row::Up => self.picker_up(),
            Row::Entry(_) => self.picker_descend(),
        }
    }

    /// Click a picker row (sets the cursor, then acts on it).
    pub fn picker_click(&mut self, row: usize) {
        if let Some(p) = self.picker.as_mut() {
            if row < p.row_count() {
                p.cursor = row;
            }
        }
        self.picker_activate();
    }

    fn picker_create_folder(&mut self, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let Some(p) = self.picker.as_mut() else {
            return;
        };
        let new = p.path.join(&name);
        if let Err(e) = std::fs::create_dir(&new) {
            p.error = Some(e.to_string());
            return;
        }
        // Open the brand-new folder as a workspace straight away — making a folder from
        // the workspace picker means "use this as my workspace", so don't make the
        // user then hunt for "open this folder".
        self.picker = None;
        self.create_workspace_at(new);
    }
}

#[cfg(test)]
mod tests {

    /// Regression: "Open folder" on a folder that is already a workspace focused
    /// a second row on the same path. `workspace.open` had the check; the picker —
    /// the way you actually open a folder — did not, so the duplicates arrived
    /// through the `+` button and then rode along in the snapshot.
    #[test]
    fn opening_an_already_open_folder_from_the_picker_focuses_it() {
        let _env = crate::persist::test_env("picker-dedupe");
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = crate::app::App::new(80, 24, tx).unwrap();
        let dir = std::env::temp_dir().join("luvus-picker-dedupe-7a2");
        std::fs::create_dir_all(&dir).unwrap();

        assert!(app.create_workspace_at(dir.clone()), "opened once");
        let count = app.workspaces.len();
        let opened = app.active_ws;
        app.active_ws = 0;

        app.open_folder_picker();
        if let Some(p) = app.picker.as_mut() {
            p.path = dir.clone();
        }
        app.picker_activate();

        assert_eq!(
            app.workspaces.len(),
            count,
            "no second row on the same folder"
        );
        assert_eq!(app.active_ws, opened, "the existing workspace is focused");
    }
    use super::*;

    #[test]
    fn repo_adds_an_open_with_worktree_row_that_shifts_the_indices() {
        let mut p = FolderPicker {
            path: PathBuf::from("/x"),
            entries: vec![Entry {
                name: "a".into(),
                is_dir: true,
            }],
            cursor: 0,
            creating: None,
            going_to: None,
            error: None,
            is_repo: false,
            show_hidden: false,
        };
        // Plain folder: [Open] [Home] [..] [a]
        assert_eq!(p.row_count(), 4);
        assert!(matches!(p.row(0), Row::OpenFolder));
        assert!(matches!(p.row(1), Row::Home));
        assert!(matches!(p.row(2), Row::Up));
        assert!(matches!(p.row(3), Row::Entry(0)));

        // Git repo: the worktree row appears at 1 and pushes the rest down.
        p.is_repo = true;
        assert_eq!(p.row_count(), 5);
        assert!(matches!(p.row(0), Row::OpenFolder));
        assert!(matches!(p.row(1), Row::OpenWorktree));
        assert!(matches!(p.row(2), Row::Home));
        assert!(matches!(p.row(3), Row::Up));
        assert!(matches!(p.row(4), Row::Entry(0)));
    }

    #[test]
    fn selecting_the_worktree_row_opens_the_branch_prompt() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(80, 24, tx).unwrap();
        app.picker = Some(FolderPicker {
            path: PathBuf::from("/tmp/some-repo"),
            entries: Vec::new(),
            cursor: 1, // the "Open with new worktree" row
            creating: None,
            going_to: None,
            error: None,
            is_repo: true,
            show_hidden: false,
        });
        app.picker_activate(); // ⏎ / click on that row
        assert!(app.picker.is_none(), "picker closes");
        assert!(app.worktree_prompt.is_some(), "branch prompt opens");
        assert_eq!(app.worktree_repo, Some(PathBuf::from("/tmp/some-repo")));
    }

    #[test]
    fn picker_browses_and_opens_a_folder() {
        let tmp = std::env::temp_dir().join(format!("luvus-picker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("readme.txt"), "hi").unwrap();

        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(80, 24, tx).unwrap();
        let workspaces_before = app.workspaces.len();

        app.open_folder_picker();
        // Point the picker at our temp dir and refresh.
        app.picker.as_mut().unwrap().path = tmp.clone();
        app.picker_refresh();
        let entries = &app.picker.as_ref().unwrap().entries;
        // Folders and files both show; the folder sorts before the file.
        assert!(entries.iter().any(|e| e.name == "sub" && e.is_dir));
        assert!(entries.iter().any(|e| e.name == "readme.txt" && !e.is_dir));
        assert!(entries[0].is_dir, "directories are listed before files");

        // Dotfiles are hidden by default; `.` toggles them on and back off.
        std::fs::write(tmp.join(".secret"), "x").unwrap();
        app.picker_refresh();
        assert!(!app.picker.as_ref().unwrap().show_hidden);
        app.handle_picker_key(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE));
        let entries = &app.picker.as_ref().unwrap().entries;
        assert!(app.picker.as_ref().unwrap().show_hidden);
        assert!(entries.iter().any(|e| e.name == ".secret"));
        app.handle_picker_key(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE));
        let entries = &app.picker.as_ref().unwrap().entries;
        assert!(!app.picker.as_ref().unwrap().show_hidden);
        assert!(!entries.iter().any(|e| e.name == ".secret"));

        // Selection survives `.` filter changes by identity, not index:
        // dotfiles sort before "readme.txt", so toggling shifts indices — the
        // highlight must stay on the same entry.
        let leading = app.picker.as_ref().unwrap().leading();
        let readme_idx = entries.iter().position(|e| e.name == "readme.txt").unwrap();
        app.picker.as_mut().unwrap().cursor = leading + readme_idx;
        app.handle_picker_key(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE));
        {
            let p = app.picker.as_ref().unwrap();
            match p.row(p.cursor) {
                Row::Entry(i) => assert_eq!(p.entries[i].name, "readme.txt"),
                other => panic!("expected readme.txt selected, got {:?}", other),
            }
        }
        // Cursor on a dotfile that gets filtered out → falls back to an
        // action row instead of silently landing on an unrelated directory.
        let secret_idx = app
            .picker
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .position(|e| e.name == ".secret")
            .unwrap();
        app.picker.as_mut().unwrap().cursor = leading + secret_idx;
        app.handle_picker_key(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE));
        let p = app.picker.as_ref().unwrap();
        assert!(!matches!(p.row(p.cursor), Row::Entry(_)));

        // Cursor 0 = "use this folder" → opens the browsed folder as a workspace.
        app.picker.as_mut().unwrap().cursor = 0;
        app.handle_picker_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.picker.is_none(), "picker closed after opening");
        assert_eq!(
            app.workspaces.len(),
            workspaces_before + 1,
            "a workspace was created"
        );
        assert_eq!(app.workspaces.last().unwrap().cwd, tmp);

        // Reopen and make a new folder: it opens as a workspace immediately (one step).
        app.open_folder_picker();
        app.picker.as_mut().unwrap().path = tmp.clone();
        app.picker_refresh();
        app.handle_picker_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        for c in "fresh".chars() {
            app.handle_picker_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        app.handle_picker_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(tmp.join("fresh").is_dir(), "new folder created");
        assert!(
            app.picker.is_none(),
            "new folder opens as a workspace (no second Enter)"
        );
        assert_eq!(app.workspaces.len(), workspaces_before + 2);
        assert_eq!(app.workspaces.last().unwrap().cwd, tmp.join("fresh"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Windows has no path above `C:\`, so `..` was a dead end and the picker
    /// stayed on whichever drive it opened on: a repo on `D:\` or on a mapped
    /// share could not be reached by browsing at all. `..` from a drive root
    /// lists the drives.
    #[cfg(windows)]
    #[test]
    fn walking_up_from_a_drive_root_lists_the_drives() {
        let _env = crate::persist::test_env("picker-drives");
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(80, 24, tx).unwrap();
        let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()) + "\\";
        app.open_folder_picker_at(PathBuf::from(&system));

        app.handle_picker_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        let p = app.picker.as_ref().expect("the picker is still open");
        assert!(p.at_drives(), "`..` from a drive root opens the drive list");
        assert!(
            matches!(p.row(0), Row::Entry(0)),
            "no `open this folder`, `home` or `..` row above the drives"
        );
        let drive = p
            .entries
            .iter()
            .position(|e| e.name.eq_ignore_ascii_case(&system))
            .expect("the system drive is listed");

        // `⏎` on a drive browses into it — the way to another letter or a share.
        app.picker.as_mut().unwrap().cursor = drive;
        app.handle_picker_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            app.picker.as_ref().expect("still open").path,
            PathBuf::from(&system),
            "browsed into the drive, not into a relative `C:` path"
        );
    }

    #[test]
    fn go_to_browses_a_path_without_opening_it() {
        let _env = crate::persist::test_env("picker-go-to");
        let tmp = std::env::temp_dir().join(format!("luvus-picker-go-{}", std::process::id()));
        let target = tmp.join("nested");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&target).unwrap();

        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(80, 24, tx).unwrap();
        let workspaces_before = app.workspaces.len();
        app.open_folder_picker_at(tmp.clone());

        app.handle_picker_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        assert_eq!(app.picker.as_ref().unwrap().going_to.as_deref(), Some(""));
        for c in target.display().to_string().chars() {
            app.handle_picker_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        app.handle_picker_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let picker = app.picker.as_ref().expect("navigation keeps picker open");
        assert_eq!(picker.path, target);
        assert!(
            picker.going_to.is_none(),
            "successful navigation exits input"
        );
        assert_eq!(
            app.workspaces.len(),
            workspaces_before,
            "Go to must not open a workspace"
        );

        // Explicit confirmation is still required.
        app.handle_picker_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.picker.is_none());
        assert_eq!(app.workspaces.len(), workspaces_before + 1);
        assert_eq!(app.workspaces.last().unwrap().cwd, target);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn go_to_keeps_invalid_paths_editable() {
        let _env = crate::persist::test_env("picker-go-to-invalid");
        let tmp =
            std::env::temp_dir().join(format!("luvus-picker-go-invalid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(80, 24, tx).unwrap();
        app.open_folder_picker_at(tmp.clone());
        app.picker_start_go_to();
        for c in "missing".chars() {
            app.handle_picker_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        app.handle_picker_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let picker = app.picker.as_ref().unwrap();
        assert_eq!(picker.path, tmp, "failed navigation keeps current folder");
        assert_eq!(picker.going_to.as_deref(), Some("missing"));
        assert!(picker.error.is_some());

        app.handle_picker_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert!(app.picker.as_ref().unwrap().error.is_none());
        app.handle_picker_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let picker = app.picker.as_ref().expect("Escape only closes Go to input");
        assert!(picker.going_to.is_none());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn home_row_and_go_to_footer_are_interactive() {
        use ratatui::backend::TestBackend;
        use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        use ratatui::Terminal;

        let _env = crate::persist::test_env("picker-home-and-footer");
        let tmp = std::env::temp_dir().join(format!("luvus-picker-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(80, 24, tx).unwrap();
        let workspaces_before = app.workspaces.len();
        app.open_folder_picker_at(tmp.clone());

        let home_row = (0..app.picker.as_ref().unwrap().row_count())
            .find(|&i| matches!(app.picker.as_ref().unwrap().row(i), Row::Home))
            .unwrap();
        app.picker.as_mut().unwrap().cursor = home_row;
        app.picker_activate();
        assert_eq!(
            app.picker.as_ref().unwrap().path,
            crate::platform::home_dir().unwrap()
        );
        assert_eq!(app.workspaces.len(), workspaces_before);

        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| crate::ui::render(f, &mut app)).unwrap();
        let screen: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(screen.contains("Home"));
        assert!(screen.contains("go to"));

        let modal = app
            .picker_rects
            .iter()
            .find_map(|(hit, rect)| (*hit == PickerHit::Modal).then_some(*rect))
            .expect("modal hit target");
        app.handle_event(AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: modal.x,
            row: modal.y,
            modifiers: KeyModifiers::NONE,
        }));
        assert!(app.picker.is_some(), "clicking modal chrome keeps it open");

        let go_to = app
            .picker_rects
            .iter()
            .find_map(|(hit, rect)| (*hit == PickerHit::Hint(KeyCode::Char('g'))).then_some(*rect))
            .expect("Go to footer hit target");
        app.handle_event(AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: go_to.x,
            row: go_to.y,
            modifiers: KeyModifiers::NONE,
        }));
        assert!(app.picker.as_ref().unwrap().going_to.is_some());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A pasted path navigates the picker instead of leaking into the pane
    /// behind it — the only way to reach a deep folder without walking there.
    #[test]
    fn pasting_a_path_jumps_the_picker_there() {
        let tmp = std::env::temp_dir().join(format!("luvus-pickpaste-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let deep = tmp.join("a").join("b");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("f.txt"), "hi").unwrap();

        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(80, 24, tx).unwrap();
        app.open_folder_picker();

        // Quoted, with trailing whitespace — what a file manager actually pastes.
        app.handle_event(crate::event::AppEvent::Paste(format!(
            "\"{}\"  ",
            deep.display()
        )));
        assert_eq!(
            app.picker.as_ref().unwrap().path,
            deep,
            "jumped to the path"
        );

        // A pasted *file* means its folder.
        app.handle_event(crate::event::AppEvent::Paste(
            deep.join("f.txt").display().to_string(),
        ));
        assert_eq!(app.picker.as_ref().unwrap().path, deep);

        // Nonsense reports itself and leaves the browsed folder alone.
        app.handle_event(crate::event::AppEvent::Paste("nope-not-a-path".into()));
        assert_eq!(app.picker.as_ref().unwrap().path, deep);
        assert!(app.picker.as_ref().unwrap().error.is_some());

        // A relative path resolves against the browsed folder — including a bare
        // filename, whose parent is the empty path and would otherwise strand
        // the picker on a folder that does not exist.
        app.handle_event(crate::event::AppEvent::Paste("f.txt".into()));
        assert_eq!(app.picker.as_ref().unwrap().path, deep);
        assert!(app.picker.as_ref().unwrap().error.is_none());
        app.picker.as_mut().unwrap().path = tmp.clone();
        app.picker_refresh();
        app.handle_event(crate::event::AppEvent::Paste("a/b".into()));
        assert_eq!(
            app.picker.as_ref().unwrap().path,
            deep,
            "walked down from here"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn paste_respects_picker_text_modes() {
        let _env = crate::persist::test_env("picker-paste-modes");
        let tmp = std::env::temp_dir().join(format!("luvus-pickmodes-{}", std::process::id()));
        let deep = tmp.join("nested");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&deep).unwrap();

        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(80, 24, tx).unwrap();
        app.open_folder_picker_at(tmp.clone());

        app.picker_start_go_to();
        app.handle_event(AppEvent::Paste(format!("\"{}\"\n", deep.display())));
        let picker = app.picker.as_ref().unwrap();
        assert_eq!(picker.path, tmp, "paste only fills the Go To field");
        assert_eq!(
            picker.going_to.as_deref(),
            Some(format!("\"{}\"", deep.display()).as_str())
        );

        app.handle_picker_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.picker.as_ref().unwrap().path, deep);

        app.handle_picker_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        app.handle_event(AppEvent::Paste("new\nfolder".into()));
        assert_eq!(
            app.picker.as_ref().unwrap().creating.as_deref(),
            Some("newfolder")
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
