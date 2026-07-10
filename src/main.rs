use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::{fs, io};

use std::time::Duration;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState},
};

// ── Entry ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    is_executable: bool,
}

fn icon_for_entry(entry: &Entry) -> (&'static str, Color) {
    // All icons use explicit \u{XXXX} Nerd Font (v2) codepoints
    const FOLDER:   &str = "\u{F07B}"; // fa-folder
    const FILE:     &str = "\u{F15B}"; // fa-file
    const RUST:     &str = "\u{E7A8}"; // dev-rust
    const JS:       &str = "\u{E74E}"; // dev-javascript
    const TS:       &str = "\u{E628}"; // seti-typescript
    const JSON:     &str = "\u{E60B}"; // seti-json
    const HTML:     &str = "\u{E736}"; // dev-html5
    const CSS:      &str = "\u{E749}"; // dev-css3
    const SCSS:     &str = "\u{E603}"; // dev-sass
    const PYTHON:   &str = "\u{E606}"; // seti-python
    const GO:       &str = "\u{E627}"; // seti-go
    const C:        &str = "\u{E61E}"; // custom-c
    const CPP:      &str = "\u{E61D}"; // custom-cpp
    const JAVA:     &str = "\u{E738}"; // dev-java
    const RUBY:     &str = "\u{E21E}"; // dev-ruby
    const SHELL:    &str = "\u{F489}"; // nf-fa-terminal
    const MARKDOWN: &str = "\u{E609}"; // seti-markdown
    const TOML:     &str = "\u{E6B2}"; // seti-config
    const YAML:     &str = "\u{E60A}"; // seti-yml
    const SQL:      &str = "\u{F1C0}"; // fa-database
    const IMAGE:    &str = "\u{F1C5}"; // fa-file-image-o
    const VIDEO:    &str = "\u{F03D}"; // fa-film
    const AUDIO:    &str = "\u{F001}"; // fa-music
    const PDF:      &str = "\u{F1C1}"; // fa-file-pdf-o
    const ARCHIVE:  &str = "\u{F1C6}"; // fa-file-archive-o
    const LOCK:     &str = "\u{F023}"; // fa-lock
    const COG:      &str = "\u{F013}"; // fa-cog
    const GIT:      &str = "\u{E702}"; // dev-git
    const DOCKER:   &str = "\u{E7B0}"; // dev-docker
    const NODE:     &str = "\u{E718}"; // dev-nodejs_small
    const TEXT:     &str = "\u{F0F6}"; // fa-file-text-o
    const LEGAL:    &str = "\u{F0E3}"; // fa-gavel
    const BINARY:   &str = "\u{F471}"; // nf-oct-file_binary
    const LIB:      &str = "\u{F1B2}"; // fa-cube
    const RUN:      &str = "\u{F0E7}"; // fa-bolt (executable)

    if entry.is_dir {
        return (FOLDER, Color::Rgb(97, 175, 239));
    }

    if entry.is_executable {
        return (RUN, Color::Rgb(80, 220, 120));
    }

    match entry.name.to_lowercase().as_str() {
        "cargo.toml"                                  => return (RUST,   Color::Rgb(222, 165, 132)),
        "cargo.lock"                                  => return (LOCK,   Color::Rgb(183, 183, 183)),
        "package.json" | "package-lock.json"          => return (NODE,   Color::Rgb(203, 120,  50)),
        ".gitignore" | ".gitmodules" | ".gitattributes" => return (GIT,  Color::Rgb(241,  80,  47)),
        "dockerfile" | "docker-compose.yml" | "docker-compose.yaml" => return (DOCKER, Color::Rgb(1, 135, 201)),
        "makefile" | "gnumakefile"                    => return (COG,    Color::Rgb(111, 193,  44)),
        "license" | "licence"                         => return (LEGAL,  Color::Rgb(240, 214,  83)),
        _ => {}
    }

    let ext = entry.path.extension().and_then(|s| s.to_str()).unwrap_or("");
    match ext {
        "rs"                              => (RUST,     Color::Rgb(222, 165, 132)),
        "js" | "mjs" | "cjs"             => (JS,       Color::Rgb(240, 214,  83)),
        "ts" | "mts" | "cts"             => (TS,       Color::Rgb( 49, 120, 198)),
        "jsx"                             => (JS,       Color::Rgb( 97, 218, 251)),
        "tsx"                             => (TS,       Color::Rgb( 49, 120, 198)),
        "json"                            => (JSON,     Color::Rgb(240, 214,  83)),
        "html" | "htm"                    => (HTML,     Color::Rgb(228,  79,  38)),
        "css"                             => (CSS,      Color::Rgb( 38, 143, 222)),
        "scss" | "sass"                   => (SCSS,     Color::Rgb(204, 102, 153)),
        "py" | "pyi"                      => (PYTHON,   Color::Rgb( 55, 118, 171)),
        "go"                              => (GO,       Color::Rgb(  1, 173, 216)),
        "c" | "h"                         => (C,        Color::Rgb( 85, 170, 255)),
        "cpp" | "cc" | "cxx" | "hpp"     => (CPP,      Color::Rgb(243,  75, 125)),
        "java"                            => (JAVA,     Color::Rgb(176, 114,  25)),
        "rb"                              => (RUBY,     Color::Rgb(204,  52,  45)),
        "sh" | "bash" | "zsh" | "fish"   => (SHELL,    Color::Rgb(121, 182, 122)),
        "md" | "mdx"                      => (MARKDOWN, Color::Rgb( 66, 165, 245)),
        "toml"                            => (TOML,     Color::Rgb(156, 175, 183)),
        "yaml" | "yml"                    => (YAML,     Color::Rgb(204, 204, 204)),
        "sql"                             => (SQL,      Color::Rgb(255, 160, 122)),
        "env"                             => (COG,      Color::Rgb(240, 214,  83)),
        "txt"                             => (TEXT,     Color::Rgb(187, 187, 187)),
        "png" | "jpg" | "jpeg" | "gif"
        | "webp" | "bmp" | "tiff" | "ico"
        | "svg"                           => (IMAGE,    Color::Rgb(167, 215,  97)),
        "mp4" | "mov" | "avi" | "mkv"
        | "webm"                          => (VIDEO,    Color::Rgb(253, 199,   0)),
        "mp3" | "wav" | "flac" | "aac"
        | "ogg"                           => (AUDIO,    Color::Rgb(  0, 188, 212)),
        "pdf"                             => (PDF,      Color::Rgb(236,  56,  50)),
        "zip" | "tar" | "gz" | "bz2"
        | "xz" | "7z"                     => (ARCHIVE,  Color::Rgb(240, 214,  83)),
        "lock"                            => (LOCK,     Color::Rgb(183, 183, 183)),
        // compiled artifacts
        "o"                               => (BINARY,   Color::Rgb(150, 120,  80)),
        "d"                               => (BINARY,   Color::Rgb(100, 100,  80)),
        "rlib" | "rmeta"                  => (LIB,      Color::Rgb(200, 120,  80)),
        "so" | "dylib" | "dll" | "a"      => (LIB,      Color::Rgb(180, 100,  60)),
        "wasm"                            => (BINARY,   Color::Rgb(100, 150, 200)),
        "pdb" | "map"                     => (BINARY,   Color::Rgb(120, 120, 120)),
        _                                 => (FILE,     Color::Rgb(180, 180, 180)),
    }
}

fn group_label(ext: &str) -> &'static str {
    match ext {
        "rs" | "toml" | "lock" | "json" | "yaml" | "yml" | "ts" | "tsx" | "js" | "jsx"
        | "html" | "css" | "scss" | "py" | "go" | "c" | "cpp" | "h" | "hpp" | "java"
        | "swift" | "kt" | "rb" | "sh" | "zsh" | "bash" | "fish" | "md" | "txt" | "gitignore"
        | "env" | "sql" => "Developer",
        "o" | "d" | "rlib" | "rmeta" | "so" | "dylib" | "dll" | "a" | "wasm" | "pdb" | "map" => "Compiled",
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" | "webp" | "bmp" | "tiff" => "Images",
        "mp4" | "mov" | "avi" | "mkv" | "webm" => "Video",
        "mp3" | "wav" | "flac" | "aac" | "ogg" => "Audio",
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" => "Documents",
        _ => "Other",
    }
}

fn read_dir_entries(path: &Path) -> Vec<Entry> {
    let mut entries: Vec<Entry> = fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let name = p.file_name()?.to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let is_dir = p.is_dir();
            let is_executable = !is_dir && p.metadata()
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false);
            Some(Entry { name, path: p, is_dir, is_executable })
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

// ── Grouped list for display ──────────────────────────────────────────────────

#[derive(Debug)]
struct GroupedEntries {
    // (group_label, entries_in_group)
    // flat index maps: index in list widget → (group_idx, entry_idx within group)
    groups: Vec<(String, Vec<usize>)>, // (label, indices into entries vec)
    entries: Vec<Entry>,
    // total selectable rows (headers are not selectable)
    row_count: usize,
    // flat_index → entry index
    row_to_entry: Vec<usize>,
}

impl GroupedEntries {
    fn build(entries: Vec<Entry>) -> Self {
        let mut folder_indices: Vec<usize> = Vec::new();
        let mut exec_indices: Vec<usize> = Vec::new();
        let mut dev_indices: Vec<usize> = Vec::new();
        let mut compiled_indices: Vec<usize> = Vec::new();
        let mut image_indices: Vec<usize> = Vec::new();
        let mut video_indices: Vec<usize> = Vec::new();
        let mut audio_indices: Vec<usize> = Vec::new();
        let mut doc_indices: Vec<usize> = Vec::new();
        let mut other_indices: Vec<usize> = Vec::new();

        for (i, e) in entries.iter().enumerate() {
            if e.is_dir {
                folder_indices.push(i);
            } else if e.is_executable {
                exec_indices.push(i);
            } else {
                let ext = e.path.extension().and_then(|s| s.to_str()).unwrap_or("");
                match group_label(ext) {
                    "Developer" => dev_indices.push(i),
                    "Compiled" => compiled_indices.push(i),
                    "Images" => image_indices.push(i),
                    "Video" => video_indices.push(i),
                    "Audio" => audio_indices.push(i),
                    "Documents" => doc_indices.push(i),
                    _ => other_indices.push(i),
                }
            }
        }

        let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
        for (label, idxs) in [
            ("Folders", folder_indices),
            ("Executables", exec_indices),
            ("Developer", dev_indices),
            ("Compiled", compiled_indices),
            ("Images", image_indices),
            ("Video", video_indices),
            ("Audio", audio_indices),
            ("Documents", doc_indices),
            ("Other", other_indices),
        ] {
            if !idxs.is_empty() {
                groups.push((label.to_string(), idxs));
            }
        }

        let mut row_to_entry: Vec<usize> = Vec::new();
        for (_, idxs) in &groups {
            for &i in idxs {
                row_to_entry.push(i);
            }
        }

        let row_count = row_to_entry.len();
        Self { groups, entries, row_count, row_to_entry }
    }

    fn list_items(&self, selected_entry_path: Option<&Path>) -> (Vec<ListItem<'static>>, usize) {
        let mut items: Vec<ListItem<'static>> = Vec::new();
        let mut selected_item_index: usize = 0;
        let label_width = if self.row_count <= 10 { 1usize } else { 2 };
        let mut row_idx = 0usize; // 0-based

        for (group_idx, (label, idxs)) in self.groups.iter().enumerate() {
            // Blank spacer before every group except the first
            if group_idx > 0 {
                items.push(ListItem::new(Line::from("")));
            }
            // Group header (indented past the jump-label gutter)
            let gutter = " ".repeat(label_width + 1);
            items.push(
                ListItem::new(Line::from(vec![
                    Span::raw(gutter),
                    Span::styled(
                        label.clone(),
                        Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
                    ),
                ]))
                .style(Style::default()),
            );

            for &ei in idxs {
                let e = &self.entries[ei];
                let (icon, icon_color) = icon_for_entry(e);
                let entry_label = if e.is_dir {
                    format!("{}/", e.name)
                } else {
                    e.name.clone()
                };
                let label_str = format!("{:>width$} ", jump_label(row_idx), width = label_width);
                let spans = vec![
                    Span::styled(label_str, Style::default().fg(Color::Rgb(80, 80, 80))),
                    Span::styled(format!("{} ", icon), Style::default().fg(icon_color)),
                    Span::raw(entry_label),
                ];

                if selected_entry_path.is_some_and(|p| p == e.path) {
                    selected_item_index = items.len();
                }

                items.push(ListItem::new(Line::from(spans)));
                row_idx += 1;
            }
        }

        (items, selected_item_index)
    }

    fn entry_at_row(&self, row: usize) -> Option<&Entry> {
        self.row_to_entry.get(row).map(|&i| &self.entries[i])
    }

    // Given a flat row index, what is the list-widget item index (accounting for headers + spacers)?
    fn list_index_for_row(&self, row: usize) -> usize {
        let mut list_idx = 0usize;
        let mut remaining = row;
        for (gi, (_, idxs)) in self.groups.iter().enumerate() {
            if gi > 0 { list_idx += 1; } // blank spacer before non-first groups
            list_idx += 1; // header
            if remaining < idxs.len() {
                return list_idx + remaining;
            }
            remaining -= idxs.len();
            list_idx += idxs.len();
        }
        list_idx
    }

    #[allow(dead_code)]
    fn entry_row_for_list_index(&self, list_idx: usize) -> Option<usize> {
        let mut idx = 0usize;
        let mut row = 0usize;
        for (gi, (_, idxs)) in self.groups.iter().enumerate() {
            if gi > 0 {
                if idx == list_idx { return None; }
                idx += 1;
            }
            if idx == list_idx { return None; }
            idx += 1;
            for _ in 0..idxs.len() {
                if idx == list_idx { return Some(row); }
                idx += 1;
                row += 1;
            }
        }
        None
    }
}

// ── Column ────────────────────────────────────────────────────────────────────

struct Column {
    path: PathBuf,
    grouped: GroupedEntries,
    selected_row: usize, // index into row_to_entry
    list_state: ListState,
}

impl Column {
    fn new(path: PathBuf) -> Self {
        let entries = read_dir_entries(&path);
        let grouped = GroupedEntries::build(entries);
        let mut list_state = ListState::default();
        if grouped.row_count > 0 {
            let li = grouped.list_index_for_row(0);
            list_state.select(Some(li));
        }
        Self { path, grouped, selected_row: 0, list_state }
    }

    fn selected_entry(&self) -> Option<&Entry> {
        self.grouped.entry_at_row(self.selected_row)
    }

    fn move_up(&mut self) {
        if self.selected_row > 0 {
            self.selected_row -= 1;
        }
        self.sync_list_state();
    }

    fn move_down(&mut self) {
        if self.grouped.row_count > 0 && self.selected_row + 1 < self.grouped.row_count {
            self.selected_row += 1;
        }
        self.sync_list_state();
    }

    fn sync_list_state(&mut self) {
        if self.grouped.row_count > 0 {
            let li = self.grouped.list_index_for_row(self.selected_row);
            self.list_state.select(Some(li));
        }
    }
}

// ── App ───────────────────────────────────────────────────────────────────────

struct App {
    columns: Vec<Column>,
    active_col: usize,
    pending_g: bool,
    pending_prefix: Option<usize>, // row offset from a qwerty prefix key
    cd_target: Option<PathBuf>,
}

fn qwerty_prefix_offset(c: char) -> Option<usize> {
    "poiuytrew".find(c).map(|i| (i + 1) * 10)
}

fn jump_label(row: usize) -> String {
    let n = row + 1;
    if n <= 10 {
        let d = if n == 10 { '0' } else { char::from_digit(n as u32, 10).unwrap() };
        d.to_string()
    } else {
        let off = n - 11;
        let prefix = "poiuytrew".chars().nth(off / 10).unwrap_or('?');
        let digit_val = off % 10 + 1;
        let d = if digit_val == 10 { '0' } else { char::from_digit(digit_val as u32, 10).unwrap() };
        format!("{}{}", prefix, d)
    }
}

impl App {
    fn new(start: PathBuf) -> Self {
        let col = Column::new(start);
        let mut app = App { columns: vec![col], active_col: 0, pending_g: false, pending_prefix: None, cd_target: None };
        // Expand into first selected dir if any
        app.maybe_push_child_column();
        app
    }

    fn maybe_push_child_column(&mut self) {
        let selected_dir = self.columns[self.active_col]
            .selected_entry()
            .filter(|e| e.is_dir)
            .map(|e| e.path.clone());

        match selected_dir {
            None => {
                // Not a dir — drop any stale preview columns
                self.columns.truncate(self.active_col + 1);
            }
            Some(path) => {
                // If the immediate next column already shows this path, keep everything to the right
                if self.columns.get(self.active_col + 1).is_some_and(|c| c.path == path) {
                    return;
                }
                self.columns.truncate(self.active_col + 1);
                self.columns.push(Column::new(path));
            }
        }
    }

    fn refresh(&mut self) {
        for col in &mut self.columns {
            let new_entries = read_dir_entries(&col.path);
            let new_grouped = GroupedEntries::build(new_entries);

            let old_name = col.grouped.entry_at_row(col.selected_row).map(|e| e.name.clone());
            col.grouped = new_grouped;

            col.selected_row = old_name
                .and_then(|name| {
                    col.grouped.row_to_entry.iter().position(|&i| col.grouped.entries[i].name == name)
                })
                .unwrap_or_else(|| col.selected_row.min(col.grouped.row_count.saturating_sub(1)));

            col.sync_list_state();
        }
    }

    fn move_up(&mut self) {
        self.columns[self.active_col].move_up();
        self.maybe_push_child_column();
    }

    fn move_down(&mut self) {
        self.columns[self.active_col].move_down();
        self.maybe_push_child_column();
    }

    fn move_right(&mut self) {
        let can_move = self.columns.get(self.active_col).and_then(|c| c.selected_entry()).is_some_and(|e| e.is_dir)
            && self.columns.len() > self.active_col + 1;
        if can_move {
            self.active_col += 1;
            self.maybe_push_child_column();
        }
    }

    fn move_left(&mut self) {
        if self.active_col > 0 {
            self.active_col -= 1;
        } else {
            // Go up to parent directory
            let current_path = self.columns[0].path.clone();
            if let Some(parent) = current_path.parent() {
                let mut parent_col = Column::new(parent.to_path_buf());
                // Pre-select the child we came from
                if let Some(row) = parent_col.grouped.row_to_entry.iter().position(|&i| {
                    parent_col.grouped.entries[i].path == current_path
                }) {
                    parent_col.selected_row = row;
                    parent_col.sync_list_state();
                }
                self.columns.insert(0, parent_col);
                // active_col stays 0, now pointing at the new parent column
            }
        }
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let col_area = area;

    let num_cols = app.columns.len();
    const COL_WIDTH: u16 = 32;
    let visible_cols = ((col_area.width / COL_WIDTH) as usize).max(1).min(num_cols);
    // Follow active_col: show it with one preview column to its right when possible
    let preferred_start = app.active_col.saturating_sub(visible_cols.saturating_sub(2));
    let start_col = preferred_start.min(num_cols.saturating_sub(visible_cols));

    let visible_count = (num_cols - start_col).min(visible_cols);
    let mut constraints: Vec<Constraint> =
        (0..visible_count).map(|_| Constraint::Length(COL_WIDTH)).collect();
    constraints.push(Constraint::Min(0)); // fill remaining space

    let col_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(col_area);

    for (vi, ci) in (start_col..num_cols).enumerate() {
        let col = &mut app.columns[ci];
        let is_active = ci == app.active_col;

        let folder_name = col
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".to_string());

        let block = Block::bordered()
            .title(Span::styled(
                format!(" {} ", folder_name),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ))
            .border_style(Style::default().fg(Color::Rgb(60, 60, 60)))
            .style(Style::default().bg(Color::Black));

        let inner = block.inner(col_chunks[vi]);
        frame.render_widget(block, col_chunks[vi]);

        // Build list items
        let selected_path = col.selected_entry().map(|e| e.path.clone());
        let (items, _) = col.grouped.list_items(selected_path.as_deref());

        let highlight_style = if is_active {
            Style::default().bg(Color::Rgb(0, 92, 197)).add_modifier(Modifier::BOLD)
        } else {
            Style::default().bg(Color::Rgb(60, 60, 60))
        };

        let list = List::new(items)
            .highlight_style(highlight_style)
            .style(Style::default().fg(Color::Rgb(200, 200, 200)));

        frame.render_stateful_widget(list, inner, &mut col.list_state);
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn open_tty() -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty")
}

fn open_in_nvim(path: &Path) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(open_tty()?, LeaveAlternateScreen, DisableMouseCapture)?;
    std::process::Command::new("nvim").arg(path).status()?;
    enable_raw_mode()?;
    execute!(open_tty()?, EnterAlternateScreen, EnableMouseCapture)?;
    Ok(())
}

fn main() -> io::Result<()> {
    let start = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap());

    enable_raw_mode()?;
    let mut tty = open_tty()?;
    execute!(tty, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(tty);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(start);

    loop {
        terminal.draw(|f| render(f, &mut app))?;

        if event::poll(Duration::from_millis(500))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => break,
                    KeyCode::Up | KeyCode::Char('k') => { app.pending_g = false; app.move_up(); }
                    KeyCode::Down | KeyCode::Char('j') => { app.pending_g = false; app.move_down(); }
                    KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('=') => { app.pending_g = false; app.move_right(); }
                    KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-') => { app.pending_g = false; app.move_left(); }
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
                        let n = offset + digit - 1; // 0-indexed
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
                    KeyCode::Char(c) if qwerty_prefix_offset(c).is_some() => {
                        app.pending_g = false;
                        app.pending_prefix = qwerty_prefix_offset(c);
                    }
                    _ => { app.pending_g = false; app.pending_prefix = None; }
                }
            }
        } else {
            app.refresh();
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    if let Some(path) = app.cd_target {
        println!("{}", path.display());
    }
    Ok(())
}
