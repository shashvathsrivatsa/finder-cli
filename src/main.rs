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

use app::{App, ClipboardEntry, ClipboardOp, CLIPBOARD_FLASH_MS, qwerty_prefix_offset};
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

fn do_paste(entry: &ClipboardEntry, dst: &Path) -> io::Result<()> {
    if entry.path == dst { return Ok(()); }
    if dst.exists() {
        if dst.is_dir() { std::fs::remove_dir_all(dst)?; }
        else            { std::fs::remove_file(dst)?; }
    }
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

    enable_raw_mode()?;
    let mut tty = open_tty()?;
    execute!(tty, EnterAlternateScreen, EnableMouseCapture, EnableFocusChange)?;
    let backend = CrosstermBackend::new(tty);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(start);

    loop {
        terminal.draw(|f| render(f, &mut app))?;

        let flash_active = app.clipboard.as_ref()
            .is_some_and(|cb| cb.set_at.elapsed().as_millis() < CLIPBOARD_FLASH_MS as u128 + 50);
        let poll_ms = if flash_active { 50 } else { 500 };
        if event::poll(Duration::from_millis(poll_ms))? {
            let ev = event::read()?;
            if matches!(ev, Event::FocusGained) { app.focused = true; continue; }
            if matches!(ev, Event::FocusLost)   { app.focused = false; continue; }
            if let Event::Key(key) = ev {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Delete confirmation intercepts all keys
                if app.confirming_delete.is_some() {
                    match key.code {
                        KeyCode::Char('y') => {
                            let path = app.confirming_delete.take().unwrap();
                            if path.is_dir() {
                                let _ = std::fs::remove_dir_all(&path);
                            } else {
                                let _ = std::fs::remove_file(&path);
                            }
                            // Move selection up if we were on the last item
                            let col = &mut app.columns[app.active_col];
                            if col.selected_row > 0 { col.selected_row -= 1; }
                            app.refresh();
                            app.maybe_push_child_column();
                        }
                        _ => { app.confirming_delete = None; }
                    }
                    continue;
                }

                // Replace confirmation intercepts all keys
                if app.confirming_replace.is_some() {
                    match key.code {
                        KeyCode::Char('y') => {
                            let (src, dst) = app.confirming_replace.take().unwrap();
                            if let Some(ref cb) = app.clipboard.clone() {
                                do_paste(&ClipboardEntry { op: cb.op.clone(), path: src, set_at: cb.set_at }, &dst).ok();
                                if cb.op == ClipboardOp::Cut { app.clipboard = None; }
                            }
                            app.refresh();
                            app.maybe_push_child_column();
                        }
                        _ => { app.confirming_replace = None; }
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

                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => break,
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.pending_g = false;
                        app.move_up();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.pending_g = false;
                        app.move_down();
                    }
                    KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('=') => {
                        app.pending_g = false;
                        app.move_right();
                    }
                    KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-') => {
                        app.pending_g = false;
                        app.move_left();
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
                        let digit = if c == '0' { 10 } else { c as usize - '0' as usize };
                        let offset = app.pending_prefix.take().unwrap_or(0);
                        let n = offset + digit - 1;
                        let col = &mut app.columns[app.active_col];
                        if n < col.grouped.row_count {
                            col.selected_row = n;
                            col.sync_list_state();
                        }
                        app.maybe_push_child_column();
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
                                open_in_nvim(&path)?;
                                terminal.clear()?;
                            }
                        }
                    }
                    KeyCode::Char(' ') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            let (path, is_dir) = (e.path.clone(), e.is_dir);
                            if is_dir {
                                app.move_right();
                            } else {
                                open_in_nvim(&path)?;
                                terminal.clear()?;
                            }
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
                    KeyCode::Char('X') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            app.clipboard = Some(ClipboardEntry { op: ClipboardOp::Cut, path: e.path.clone(), set_at: std::time::Instant::now() });
                        }
                    }
                    KeyCode::Char('C') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            app.clipboard = Some(ClipboardEntry { op: ClipboardOp::Copy, path: e.path.clone(), set_at: std::time::Instant::now() });
                        }
                    }
                    KeyCode::Char('V') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        if let Some(ref cb) = app.clipboard.clone() {
                            if let Some(filename) = cb.path.file_name() {
                                let col = &app.columns[app.active_col];
                                let dest_dir = col.selected_entry()
                                    .filter(|e| e.is_dir)
                                    .map(|e| e.path.clone())
                                    .unwrap_or_else(|| col.path.clone());
                                let dst = dest_dir.join(filename);
                                if dst.exists() && dst != cb.path {
                                    app.confirming_replace = Some((cb.path.clone(), dst));
                                } else {
                                    let is_cut = cb.op == ClipboardOp::Cut;
                                    do_paste(cb, &dst).ok();
                                    if is_cut { app.clipboard = None; }
                                    app.refresh();
                                    app.maybe_push_child_column();
                                }
                            }
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
                    KeyCode::Char('K') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            let dst = copy_dest(&e.path);
                            if e.is_dir { copy_dir(&e.path, &dst).ok(); }
                            else { std::fs::copy(&e.path, &dst).ok(); }
                            app.refresh();
                        }
                    }
                    KeyCode::Char('D') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        if let Some(e) = col.grouped.entry_at_row(col.selected_row) {
                            app.confirming_delete = Some(e.path.clone());
                        }
                    }
                    KeyCode::Char('d') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        // Create inside selected dir, or in current column if a file is selected
                        let col = &app.columns[app.active_col];
                        let base_path = col.selected_entry()
                            .filter(|e| e.is_dir)
                            .map(|e| e.path.clone())
                            .unwrap_or_else(|| col.path.clone());
                        let placeholder = (0u32..).map(|i| {
                            if i == 0 { "untitled".to_string() } else { format!("untitled {}", i) }
                        }).find(|name| !base_path.join(name).exists()).unwrap();
                        let new_dir = base_path.join(&placeholder);
                        if std::fs::create_dir(&new_dir).is_ok() {
                            let selected_is_dir = app.columns[app.active_col]
                                .selected_entry().is_some_and(|e| e.is_dir);
                            if selected_is_dir {
                                app.maybe_push_child_column(); // ensure child column exists first
                                app.move_right();
                            }
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
                    KeyCode::Char('T') => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                        let col = &app.columns[app.active_col];
                        let base_path = col.selected_entry()
                            .filter(|e| e.is_dir)
                            .map(|e| e.path.clone())
                            .unwrap_or_else(|| col.path.clone());
                        let placeholder = (0u32..).map(|i| {
                            if i == 0 { "untitled".to_string() } else { format!("untitled {}", i) }
                        }).find(|name| !base_path.join(name).exists()).unwrap();
                        let new_file = base_path.join(&placeholder);
                        if std::fs::File::create(&new_file).is_ok() {
                            let selected_is_dir = app.columns[app.active_col]
                                .selected_entry().is_some_and(|e| e.is_dir);
                            if selected_is_dir {
                                app.maybe_push_child_column();
                                app.move_right();
                            }
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
                    KeyCode::Char(c) if qwerty_prefix_offset(c).is_some() => {
                        app.pending_g = false;
                        app.pending_prefix = qwerty_prefix_offset(c);
                    }
                    _ => {
                        app.pending_g = false;
                        app.pending_prefix = None;
                    }
                }
            }
        } else {
            app.refresh();
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture, DisableFocusChange)?;
    terminal.show_cursor()?;
    if let Some(path) = app.cd_target {
        println!("{}", path.display());
    }
    Ok(())
}
