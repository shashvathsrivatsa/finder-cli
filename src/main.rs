mod app;
mod column;
mod entry;
mod grouped;
mod rename;
mod ui;

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, DisableFocusChange, EnableFocusChange, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use app::{App, ClipboardEntry, ClipboardOp, PaneInfo, CLIPBOARD_FLASH_MS, PAGE_JUMP};
use rename::{RenameMode, RenameState};
use ui::render;

fn copy_dest(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or(Path::new("."));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let ext  = path.extension().and_then(|s| s.to_str());
    let make = |n: u32| {
        let suffix = if n == 1 { "copy".to_string() } else { format!("copy {}", n) };
        if let Some(e) = ext { format!("{} {}.{}", stem, suffix, e) } else { format!("{} {}", stem, suffix) }
    };
    (1u32..).map(|n| parent.join(make(n))).find(|p| !p.exists()).unwrap()
}

fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
    std::fs::create_dir(dst)?;
    for entry in std::fs::read_dir(src)?.flatten() {
        let dst_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() { copy_dir(&entry.path(), &dst_path)?; }
        else { std::fs::copy(&entry.path(), &dst_path)?; }
    }
    Ok(())
}

fn list_panes() -> Vec<PaneInfo> {
    let current_pane = std::env::var("TMUX_PANE").unwrap_or_default();
    let current_session = std::process::Command::new("tmux")
        .args(["display-message", "-p", "#S"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let Ok(out) = std::process::Command::new("tmux")
        .args(["list-panes", "-a", "-F", "#{pane_id}\t#{session_name}\t#{session_name}:#{window_index}.#{pane_index}\t#{pane_current_command}"])
        .output() else { return vec![]; };
    let mut panes: Vec<PaneInfo> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(4, '\t');
            let id      = parts.next()?.to_string();
            let session = parts.next()?.to_string();
            let coord   = parts.next()?.to_string();
            let cmd     = parts.next()?.trim().to_string();
            if id == current_pane { return None; }
            if cmd != "nvim" { return None; }
            let same_session = session == current_session;
            Some(PaneInfo { id, label: format!("{}  {}", coord, cmd), same_session })
        })
        .collect();
    // current session first
    panes.sort_by_key(|p| !p.same_session);
    panes
}

fn open_in_linked_pane(pane_id: &str, path: &Path) {
    let path_str = path.to_string_lossy();
    let _ = std::process::Command::new("tmux")
        .args(["send-keys", "-t", pane_id, &format!(":e {}\r", path_str)])
        .status();
    let _ = std::process::Command::new("tmux")
        .args(["select-pane", "-t", pane_id])
        .status();
}

fn unique_dest(dir: &Path, filename: &std::ffi::OsStr, src: &Path, is_move: bool) -> PathBuf {
    let candidate = dir.join(filename);
    if !candidate.exists() || (is_move && candidate == src) {
        return candidate;
    }
    let name = Path::new(filename);
    let stem = name.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let ext = name.extension().and_then(|s| s.to_str());
    let mut i = 0usize;
    loop {
        let new_name = if i == 0 {
            if let Some(e) = ext { format!("{} copy.{}", stem, e) } else { format!("{} copy", stem) }
        } else {
            if let Some(e) = ext { format!("{} copy {}.{}", stem, i + 1, e) } else { format!("{} copy {}", stem, i + 1) }
        };
        let candidate = dir.join(&new_name);
        if !candidate.exists() { return candidate; }
        i += 1;
    }
}

fn do_paste(entry: &ClipboardEntry, dst: &Path) -> io::Result<()> {
    if entry.path == dst { return Ok(()); }
    match entry.op {
        ClipboardOp::Cut  => std::fs::rename(&entry.path, dst)?,
        ClipboardOp::Copy => {
            if entry.path.is_dir() { copy_dir(&entry.path, dst)?; }
            else                   { std::fs::copy(&entry.path, dst).map(|_| ())?; }
        }
    }
    Ok(())
}

fn open_tty() -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty")
}

fn load_default_app_exts() -> std::collections::HashSet<String> {
    let exe_dir = std::env::current_exe().ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    // Look next to the binary, then next to Cargo.toml (dev), then CWD
    let candidates = [
        exe_dir.as_deref().map(|d| d.join("defaults")),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("defaults")),
        Some(PathBuf::from("defaults")),
    ];
    for path in candidates.into_iter().flatten() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            return text.lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| l.to_lowercase())
                .collect();
        }
    }
    std::collections::HashSet::new()
}

fn open_in_default_app(path: &Path) {
    let _ = std::process::Command::new("open").arg(path).status();
}

fn open_in_nvim(path: &Path) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(open_tty()?, LeaveAlternateScreen, DisableMouseCapture)?;
    let tty_in  = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    let tty_out = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    let tty_err = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    std::process::Command::new("nvim")
        .arg(path)
        .stdin(tty_in)
        .stdout(tty_out)
        .stderr(tty_err)
        .status()?;
    enable_raw_mode()?;
    execute!(open_tty()?, EnterAlternateScreen, EnableMouseCapture, EnableFocusChange)?;
    Ok(())
}

fn main() -> io::Result<()> {
    let start = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap());

    let mut shelf_proc = std::process::Command::new("shelf")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn().ok();

    enable_raw_mode()?;
    let mut tty = open_tty()?;
    execute!(tty, EnterAlternateScreen, EnableMouseCapture, EnableFocusChange)?;
    let backend = CrosstermBackend::new(tty);
    let mut terminal = Terminal::new(backend)?;

    let default_app_exts = load_default_app_exts();
    let mut app = App::new(start);

    let mut last_refresh = std::time::Instant::now();
    const IDLE_REFRESH_MS: u64 = 5_000;
    let mut needs_redraw = true;

    loop {
        if needs_redraw {
            terminal.draw(|f| render(f, &mut app))?;
            needs_redraw = false;
        }

        let flash_active = app.clipboard.as_ref()
            .is_some_and(|cb| cb.set_at.elapsed().as_millis() < CLIPBOARD_FLASH_MS as u128 + 50);
        let poll_ms = if flash_active { 50 } else { 500 };
        if event::poll(Duration::from_millis(poll_ms))? {
            let ev = event::read()?;
            if matches!(ev, Event::FocusGained) { app.focused = true; needs_redraw = true; continue; }
            if matches!(ev, Event::FocusLost)   { app.focused = false; needs_redraw = true; continue; }
            if let Event::Key(key) = ev {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                needs_redraw = true;

                // Delete confirmation intercepts all keys
                if app.confirming_delete.is_some() {
                    match key.code {
                        KeyCode::Char('y') => {
                            app.confirming_delete = None;
                            let paths = std::mem::take(&mut app.pending_deletes);
                            for path in &paths {
                                if path.is_dir() { let _ = std::fs::remove_dir_all(path); }
                                else { let _ = std::fs::remove_file(path); }
                            }
                            app.selection.clear(); app.selection_anchor = None; app.select_mode = false;
                            app.refresh();
                            let col = &mut app.columns[app.active_col];
                            if col.selected_row >= col.grouped.row_count && col.selected_row > 0 {
                                col.selected_row -= 1;
                            }
                            app.maybe_push_child_column();
                        }
                        _ => { app.confirming_delete = None; app.pending_deletes.clear(); }
                    }
                    continue;
                }

                // Pane picker intercepts all keys
                if let Some((ref panes, ref mut sel)) = app.pane_picker {
                    let panes = panes.clone();
                    let count = panes.len();
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') => { app.pane_picker = None; }
                        KeyCode::Char('j') | KeyCode::Down  => { *sel = (*sel + 1).min(count.saturating_sub(1)); }
                        KeyCode::Char('k') | KeyCode::Up    => { *sel = sel.saturating_sub(1); }
                        KeyCode::Enter => {
                            let chosen = panes[*sel].clone();
                            app.linked_pane = Some(chosen);
                            app.pane_picker = None;
                        }
                        KeyCode::Char('u') => {
                            app.linked_pane = None;
                            app.pane_picker = None;
                        }
                        _ => {}
                    }
                    continue;
                }

                // Rename mode intercepts all keys
                if let Some(ref mut rs) = app.renaming {
                    match rs.mode {
                        RenameMode::Insert => match key.code {
                            KeyCode::Esc => rs.enter_normal(),
                            KeyCode::Enter => {
                                let col = &app.columns[app.active_col];
                                if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                                    let new_name = rs.text.clone();
                                    let old_path = e.path.clone();
                                    if !new_name.is_empty() && new_name != e.name {
                                        let new_path = old_path.parent().unwrap().join(&new_name);
                                        let _ = std::fs::rename(&old_path, &new_path);
                                    }
                                }
                                app.renaming = None;
                                app.refresh();
                            }
                            KeyCode::Backspace => rs.backspace(),
                            KeyCode::Left     => rs.move_left(),
                            KeyCode::Right    => rs.move_right(),
                            KeyCode::Char(c)  => rs.insert_char(c),
                            _ => {}
                        },
                        RenameMode::Visual => {
                            match key.code {
                                KeyCode::Esc | KeyCode::Char('v') => { rs.mode = rename::RenameMode::Normal; }
                                KeyCode::Char('h') | KeyCode::Left  => rs.move_left(),
                                KeyCode::Char('l') | KeyCode::Right => rs.move_right(),
                                KeyCode::Char('w') => rs.move_word_forward(),
                                KeyCode::Char('b') => rs.move_word_backward(),
                                KeyCode::Char('e') => rs.move_word_end(),
                                KeyCode::Char('0') => rs.move_line_start(),
                                KeyCode::Char('$') => rs.move_line_end(),
                                KeyCode::Char('d') | KeyCode::Char('x') => {
                                    rs.delete_visual_selection();
                                    rs.mode = rename::RenameMode::Normal;
                                }
                                KeyCode::Char('c') => {
                                    rs.delete_visual_selection();
                                    rs.enter_insert_before();
                                }
                                _ => {}
                            }
                        }
                        RenameMode::Normal => {
                            let confirm_rename = |rs: &RenameState, app: &mut App| {
                                let col = &app.columns[app.active_col];
                                if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                                    let new_name = rs.text.clone();
                                    let old_path = e.path.clone();
                                    if !new_name.is_empty() && new_name != e.name {
                                        let new_path = old_path.parent().unwrap().join(&new_name);
                                        let _ = std::fs::rename(&old_path, &new_path);
                                    }
                                }
                            };

                            // Consume pending multi-key sequences
                            let pending = rs.pending.clone();
                            match (pending.as_str(), key.code) {
                                // ── r<char>: replace ──────────────────────────
                                ("r", KeyCode::Char(c)) => { rs.replace_char(c); rs.pending.clear(); }
                                ("r", _) => { rs.pending.clear(); }

                                // ── d<motion> ─────────────────────────────────
                                ("d", KeyCode::Char('d')) => { rs.clear_text(); rs.pending.clear(); }
                                ("d", KeyCode::Char('w')) => { rs.delete_word_forward(); rs.pending.clear(); }
                                ("d", KeyCode::Char('e')) => { rs.delete_to_word_end(); rs.pending.clear(); }
                                ("d", KeyCode::Char('b')) => { rs.delete_to_word_start(); rs.pending.clear(); }
                                ("d", KeyCode::Char('0')) => { rs.delete_to_line_start(); rs.pending.clear(); }
                                ("d", KeyCode::Char('$')) => { rs.delete_to_line_end(); rs.pending.clear(); }
                                ("d", KeyCode::Char('i')) => { rs.pending = "di".into(); }
                                ("d", _) => { rs.pending.clear(); }

                                // ── di<object> ────────────────────────────────
                                ("di", KeyCode::Char('w')) => { rs.delete_inner_word(); rs.pending.clear(); }
                                ("di", _) => { rs.pending.clear(); }

                                // ── c<motion> ─────────────────────────────────
                                ("c", KeyCode::Char('c')) => { rs.clear_text(); rs.enter_insert_before(); rs.pending.clear(); }
                                ("c", KeyCode::Char('w')) | ("c", KeyCode::Char('e')) => { rs.delete_to_word_end(); rs.enter_insert_before(); rs.pending.clear(); }
                                ("c", KeyCode::Char('b')) => { rs.delete_to_word_start(); rs.enter_insert_before(); rs.pending.clear(); }
                                ("c", KeyCode::Char('0')) => { rs.delete_to_line_start(); rs.enter_insert_before(); rs.pending.clear(); }
                                ("c", KeyCode::Char('$')) => { rs.delete_to_line_end(); rs.enter_insert_before(); rs.pending.clear(); }
                                ("c", KeyCode::Char('i')) => { rs.pending = "ci".into(); }
                                ("c", _) => { rs.pending.clear(); }

                                // ── ci<object> ────────────────────────────────
                                ("ci", KeyCode::Char('w')) => { rs.delete_inner_word(); rs.enter_insert_before(); rs.pending.clear(); }
                                ("ci", _) => { rs.pending.clear(); }

                                // ── no pending: immediate commands ────────────
                                (_, KeyCode::Esc) => { rs.pending.clear(); app.renaming = None; }
                                (_, KeyCode::Enter) => {
                                    let rs_ref = app.renaming.as_ref().unwrap();
                                    let text = rs_ref.text.clone();
                                    let col = &app.columns[app.active_col];
                                    if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                                        if !text.is_empty() && text != e.name {
                                            let new_path = e.path.parent().unwrap().join(&text);
                                            let _ = std::fs::rename(&e.path, &new_path);
                                        }
                                    }
                                    app.renaming = None;
                                    app.refresh();
                                }
                                (_, KeyCode::Char('h')) | (_, KeyCode::Left)  => rs.move_left(),
                                (_, KeyCode::Char('l')) | (_, KeyCode::Right) => rs.move_right(),
                                (_, KeyCode::Char('w')) => rs.move_word_forward(),
                                (_, KeyCode::Char('b')) => rs.move_word_backward(),
                                (_, KeyCode::Char('e')) => rs.move_word_end(),
                                (_, KeyCode::Char('0')) => rs.move_line_start(),
                                (_, KeyCode::Char('$')) => rs.move_line_end(),
                                (_, KeyCode::Char('x')) => rs.delete_at_cursor(),
                                (_, KeyCode::Char('X')) => rs.backspace(),
                                (_, KeyCode::Char('s')) => { rs.delete_at_cursor(); rs.enter_insert_before(); }
                                (_, KeyCode::Char('D')) => rs.delete_to_line_end(),
                                (_, KeyCode::Char('C')) => { rs.delete_to_line_end(); rs.enter_insert_before(); }
                                (_, KeyCode::Char('S')) => { rs.clear_text(); rs.enter_insert_before(); }
                                (_, KeyCode::Char('i')) => rs.enter_insert_before(),
                                (_, KeyCode::Char('a')) => rs.enter_insert_after(),
                                (_, KeyCode::Char('I')) => rs.enter_insert_start(),
                                (_, KeyCode::Char('A')) => rs.enter_insert_end(),
                                (_, KeyCode::Char('v')) => rs.enter_visual(),
                                (_, KeyCode::Char('d')) => rs.pending = "d".into(),
                                (_, KeyCode::Char('c')) => rs.pending = "c".into(),
                                (_, KeyCode::Char('r')) => rs.pending = "r".into(),
                                _ => { rs.pending.clear(); }
                            }
                            // suppress unused warning
                            let _ = confirm_rename;
                        }
                    }
                    continue;
                }

                // Goto mode intercepts all keys
                if app.goto_query.is_some() {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => { app.goto_query = None; }
                        KeyCode::Backspace => {
                            if let Some(ref mut q) = app.goto_query {
                                q.pop();
                            }
                        }
                        KeyCode::Char(c) => {
                            if let Some(ref mut q) = app.goto_query {
                                q.push(c);
                            }
                        }
                        _ => {}
                    }
                    if let Some(ref q) = app.goto_query.clone() {
                        if !q.is_empty() {
                            let col = &mut app.columns[app.active_col];
                            let ql = q.to_lowercase();
                            if let Some(row) = (0..col.grouped.row_count).find(|&r| {
                                col.grouped.entry_at_row(r)
                                    .is_some_and(|e| e.name.to_lowercase().starts_with(&ql))
                            }) {
                                col.selected_row = row;
                                col.sync_list_state();
                                app.maybe_push_child_column();
                            }
                        }
                    }
                    needs_redraw = true;
                    continue;
                }

                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        if app.select_mode {
                            app.select_mode = false;
                            // keep selection so user can still X/C after exiting select mode
                        } else {
                            break;
                        }
                    }
                    KeyCode::Char('V') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        if app.select_mode {
                            app.select_mode = false;
                            app.selection.clear();
                            app.selection_anchor = None;
                        } else {
                            app.select_mode = true;
                        }
                    }
                    KeyCode::Char('.') if app.select_mode => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        let row = col.selected_row;
                        if let Some(e) = col.grouped.entry_at_row(row) {
                            let path = e.path.clone();
                            if app.selection.contains(&path) {
                                app.selection.remove(&path);
                            } else {
                                app.selection.insert(path);
                                app.selection_anchor = Some(row);
                            }
                        }
                    }
                    KeyCode::Char(' ') | KeyCode::Char(',') if app.select_mode => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        let current_row = col.selected_row;
                        let anchor = app.selection_anchor.unwrap_or(current_row);
                        let lo = anchor.min(current_row);
                        let hi = anchor.max(current_row);
                        let paths: Vec<_> = (lo..=hi)
                            .filter_map(|r| col.grouped.entry_at_row(r).map(|e| e.path.clone()))
                            .collect();
                        for path in paths {
                            app.selection.insert(path);
                        }
                        app.selection_anchor = Some(current_row);
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.pending_g = false;
                        app.move_up();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.pending_g = false;
                        app.move_down();
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.pending_g = false;
                        app.columns[app.active_col].move_by(PAGE_JUMP as isize);
                        app.maybe_push_child_column();
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.pending_g = false;
                        app.columns[app.active_col].move_by(-(PAGE_JUMP as isize));
                        app.maybe_push_child_column();
                    }
                    KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('=') if !key.modifiers.contains(KeyModifiers::ALT) => {
                        app.pending_g = false;
                        app.move_right();
                    }
                    KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-') if !key.modifiers.contains(KeyModifiers::ALT) => {
                        app.pending_g = false;
                        app.move_left();
                    }
                    KeyCode::Char('=') if key.modifiers.contains(KeyModifiers::ALT) => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let vh = app.col_viewport_height;
                        app.columns[app.active_col].scroll_by(5, vh);
                    }
                    KeyCode::Char('-') if key.modifiers.contains(KeyModifiers::ALT) => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let vh = app.col_viewport_height;
                        app.columns[app.active_col].scroll_by(-5, vh);
                    }
                    KeyCode::Char('/') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        app.goto_query = Some(String::new());
                    }
                    KeyCode::Char('n') => {
                        app.pending_g = false;
                        app.columns.drain(0..app.active_col);
                        app.active_col = 0;
                    }
                    KeyCode::Char('G') => {
                        app.pending_g = false;
                        let col = &mut app.columns[app.active_col];
                        if col.grouped.row_count > 0 {
                            col.selected_row = col.grouped.row_count - 1;
                            col.sync_list_state();
                        }
                        app.maybe_push_child_column();
                    }
                    KeyCode::Char('g') => {
                        app.pending_prefix = None;
                        if app.pending_g {
                            app.pending_g = false;
                            let col = &mut app.columns[app.active_col];
                            col.selected_row = 0;
                            col.sync_list_state();
                            app.maybe_push_child_column();
                        } else {
                            app.pending_g = true;
                        }
                    }
                    KeyCode::Char(c @ '0'..='9') => {
                        app.pending_g = false;
                        let d = c as usize - '0' as usize;
                        let n = app.pending_prefix.unwrap_or(0) * 10 + d;
                        app.pending_digits += 1;
                        app.pending_prefix = Some(n);
                        let col = &mut app.columns[app.active_col];
                        let lw = crate::grouped::label_width(col.grouped.row_count);
                        if app.pending_digits >= lw {
                            app.pending_prefix = None;
                            app.pending_digits = 0;
                            let row = n.saturating_sub(1);
                            if row < col.grouped.row_count {
                                col.selected_row = row;
                                col.sync_list_state();
                            }
                            app.maybe_push_child_column();
                        }
                    }
                    KeyCode::Enter => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            let (path, is_dir) = (e.path.clone(), e.is_dir);
                            if is_dir {
                                app.cd_target = Some(path);
                                break;
                            } else {
                                let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                                if default_app_exts.contains(&ext) {
                                    open_in_default_app(&path);
                                } else if let Some(ref pane) = app.linked_pane.clone() {
                                    open_in_linked_pane(&pane.id, &path);
                                } else {
                                    open_in_nvim(&path)?;
                                    terminal.clear()?;
                                }
                            }
                        }
                    }
                    KeyCode::Char('P') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let panes = list_panes();
                        if !panes.is_empty() {
                            let sel = app.linked_pane.as_ref()
                                .and_then(|lp| panes.iter().position(|p| p.id == lp.id))
                                .unwrap_or(0);
                            app.pane_picker = Some((panes, sel));
                        }
                    }
                    KeyCode::Char('R') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            app.renaming = Some(RenameState::new(&e.name));
                        }
                    }
                    KeyCode::Char('m') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        let paths: Vec<PathBuf> = if !app.selection.is_empty() {
                            let mut v: Vec<PathBuf> = app.selection.iter().cloned().collect();
                            v.sort();
                            v
                        } else if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            vec![e.path.clone()]
                        } else { vec![] };
                        if !paths.is_empty() {
                            let primary = paths[0].clone();
                            app.clipboard = Some(ClipboardEntry { op: ClipboardOp::Cut, path: primary, paths, set_at: std::time::Instant::now() });
                            app.selection.clear(); app.selection_anchor = None;
                        }
                    }
                    KeyCode::Char('y') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        let paths: Vec<PathBuf> = if !app.selection.is_empty() {
                            let mut v: Vec<PathBuf> = app.selection.iter().cloned().collect();
                            v.sort();
                            v
                        } else if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            vec![e.path.clone()]
                        } else { vec![] };
                        if !paths.is_empty() {
                            let primary = paths[0].clone();
                            app.clipboard = Some(ClipboardEntry { op: ClipboardOp::Copy, path: primary, paths, set_at: std::time::Instant::now() });
                            app.selection.clear(); app.selection_anchor = None;
                        }
                    }
                    KeyCode::Char('p') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        if let Some(ref cb) = app.clipboard.clone() {
                            let dest_dir = app.columns[app.active_col].path.clone();
                            let is_cut = cb.op == ClipboardOp::Cut;
                            for src in &cb.paths {
                                if let Some(filename) = src.file_name() {
                                    let single = ClipboardEntry { op: cb.op.clone(), path: src.clone(), paths: vec![src.clone()], set_at: cb.set_at };
                                    let dst = unique_dest(&dest_dir, filename, src, is_cut);
                                    do_paste(&single, &dst).ok();
                                }
                            }
                            if is_cut { app.clipboard = None; }
                            app.selection.clear(); app.selection_anchor = None; app.select_mode = false;
                            app.refresh();
                            app.maybe_push_child_column();
                        }
                    }
                    KeyCode::Char('x') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            std::process::Command::new("open").arg(&e.path).spawn().ok();
                        }
                    }
                    KeyCode::Tab => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        let paths: Vec<_> = if !app.selection.is_empty() {
                            app.selection.iter().cloned().collect()
                        } else if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            vec![e.path.clone()]
                        } else { vec![] };
                        for path in paths {
                            std::process::Command::new("shelf-add")
                                .arg(&path)
                                .spawn().ok();
                        }
                    }
                    KeyCode::Char('f') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            std::process::Command::new("open")
                                .arg("-R")
                                .arg(&e.path)
                                .spawn()
                                .ok();
                        }
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            let path = e.path.to_string_lossy().into_owned();
                            let mut child = std::process::Command::new("pbcopy")
                                .stdin(std::process::Stdio::piped())
                                .spawn()
                                .ok();
                            if let Some(ref mut c) = child {
                                if let Some(stdin) = c.stdin.as_mut() {
                                    use std::io::Write;
                                    let _ = stdin.write_all(path.as_bytes());
                                }
                            }
                        }
                    }
                    KeyCode::Char('K') if key.modifiers.contains(KeyModifiers::ALT) => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            let dst = copy_dest(&e.path);
                            if e.is_dir { copy_dir(&e.path, &dst).ok(); }
                            else { std::fs::copy(&e.path, &dst).ok(); }
                            let dst_name = dst.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                            app.refresh();
                            let col = &mut app.columns[app.active_col];
                            if let Some(row) = col.grouped.row_to_entry.iter().position(|&i| {
                                col.grouped.entries[i].name == dst_name
                            }) {
                                col.selected_row = row;
                                col.sync_list_state();
                            }
                        }
                    }
                    KeyCode::Char('D') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        if !app.selection.is_empty() {
                            let mut paths: Vec<PathBuf> = app.selection.iter().cloned().collect();
                            paths.sort();
                            let count = paths.len();
                            app.pending_deletes = paths;
                            app.confirming_delete = Some(PathBuf::from(format!("{} items", count)));
                        } else {
                            let col = &app.columns[app.active_col];
                            if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                                app.pending_deletes = vec![e.path.clone()];
                                app.confirming_delete = Some(e.path.clone());
                            }
                        }
                    }
                    KeyCode::Char('d') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        let base_path = col.path.clone();
                        let placeholder = (0u32..).map(|i| {
                            if i == 0 { "untitled".to_string() } else { format!("untitled {}", i) }
                        }).find(|name| !base_path.join(name).exists()).unwrap();
                        let new_dir = base_path.join(&placeholder);
                        if std::fs::create_dir(&new_dir).is_ok() {
                            app.refresh();
                            // Select the new dir in the target column and enter rename mode
                            let col = &mut app.columns[app.active_col];
                            if let Some(row) = col.grouped.row_to_entry.iter().position(|&i| {
                                col.grouped.entries[i].name == placeholder
                            }) {
                                col.selected_row = row;
                                col.sync_list_state();
                            }
                            app.renaming = Some(RenameState {
                                text: String::new(),
                                cursor: 0,
                                mode: RenameMode::Insert,
                                pending: String::new(),
                                visual_anchor: 0,
                            });
                            app.maybe_push_child_column();
                        }
                    }
                    KeyCode::Char('%') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        let base_path = col.path.clone();
                        let placeholder = (0u32..).map(|i| {
                            if i == 0 { "untitled".to_string() } else { format!("untitled {}", i) }
                        }).find(|name| !base_path.join(name).exists()).unwrap();
                        let new_file = base_path.join(&placeholder);
                        if std::fs::File::create(&new_file).is_ok() {
                            app.refresh();
                            let col = &mut app.columns[app.active_col];
                            if let Some(row) = col.grouped.row_to_entry.iter().position(|&i| {
                                col.grouped.entries[i].name == placeholder
                            }) {
                                col.selected_row = row;
                                col.sync_list_state();
                            }
                            app.renaming = Some(RenameState {
                                text: String::new(),
                                cursor: 0,
                                mode: RenameMode::Insert,
                                pending: String::new(),
                                visual_anchor: 0,
                            });
                            app.maybe_push_child_column();
                        }
                    }
                    _ => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        app.pending_digits = 0;
                    }
                }
            }
        } else {
            if flash_active {
                needs_redraw = true;
            }
            if last_refresh.elapsed().as_millis() >= IDLE_REFRESH_MS as u128 {
                app.refresh();
                last_refresh = std::time::Instant::now();
                needs_redraw = true;
            }
        }
    }

    if let Some(ref mut p) = shelf_proc { p.kill().ok(); }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture, DisableFocusChange)?;
    terminal.show_cursor()?;
    if let Some(path) = app.cd_target {
        println!("{}", path.display());
    }
    Ok(())
}
