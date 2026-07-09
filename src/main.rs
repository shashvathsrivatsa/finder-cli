use std::path::{Path, PathBuf};
use std::{fs, io};

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
    widgets::{Block, List, ListItem, ListState, Paragraph},
};

// ── Entry ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

fn group_label(ext: &str) -> &'static str {
    match ext {
        "rs" | "toml" | "lock" | "json" | "yaml" | "yml" | "ts" | "tsx" | "js" | "jsx"
        | "html" | "css" | "scss" | "py" | "go" | "c" | "cpp" | "h" | "hpp" | "java"
        | "swift" | "kt" | "rb" | "sh" | "zsh" | "bash" | "fish" | "md" | "txt" | "gitignore"
        | "env" | "sql" => "Developer",
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
            Some(Entry { name, path: p, is_dir })
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
        let mut dev_indices: Vec<usize> = Vec::new();
        let mut image_indices: Vec<usize> = Vec::new();
        let mut video_indices: Vec<usize> = Vec::new();
        let mut audio_indices: Vec<usize> = Vec::new();
        let mut doc_indices: Vec<usize> = Vec::new();
        let mut other_indices: Vec<usize> = Vec::new();

        for (i, e) in entries.iter().enumerate() {
            if e.is_dir {
                folder_indices.push(i);
            } else {
                let ext = e.path.extension().and_then(|s| s.to_str()).unwrap_or("");
                match group_label(ext) {
                    "Developer" => dev_indices.push(i),
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
            ("Developer", dev_indices),
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
        let mut running = 0usize;

        for (label, idxs) in &self.groups {
            // Group header (dim, small)
            items.push(
                ListItem::new(Line::from(vec![Span::styled(
                    label.clone(),
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
                )]))
                .style(Style::default()),
            );

            for &ei in idxs {
                let e = &self.entries[ei];
                let icon = if e.is_dir { " " } else { " " };
                let has_children = e.is_dir;

                let mut spans = vec![Span::raw(format!("{}{}", icon, e.name))];
                if has_children {
                    spans.push(Span::styled(
                        "  ›",
                        Style::default().fg(Color::DarkGray),
                    ));
                }

                if selected_entry_path.is_some_and(|p| p == e.path) {
                    selected_item_index = items.len();
                }

                items.push(ListItem::new(Line::from(spans)));
                running += 1;
                let _ = running;
            }
        }

        (items, selected_item_index)
    }

    fn entry_at_row(&self, row: usize) -> Option<&Entry> {
        self.row_to_entry.get(row).map(|&i| &self.entries[i])
    }

    // Given a flat row index, what is the list-widget item index (skipping headers)?
    fn list_index_for_row(&self, row: usize) -> usize {
        let mut list_idx = 0usize;
        let mut remaining = row;
        for (_, idxs) in &self.groups {
            list_idx += 1; // header
            if remaining < idxs.len() {
                return list_idx + remaining;
            }
            remaining -= idxs.len();
            list_idx += idxs.len();
        }
        list_idx
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
}

impl App {
    fn new(start: PathBuf) -> Self {
        let col = Column::new(start);
        let mut app = App { columns: vec![col], active_col: 0 };
        // Expand into first selected dir if any
        app.maybe_push_child_column();
        app
    }

    fn maybe_push_child_column(&mut self) {
        // Keep columns to the right of active_col pruned first
        self.columns.truncate(self.active_col + 1);

        let Some(entry) = self.columns[self.active_col].selected_entry() else {
            return;
        };
        if entry.is_dir {
            let child_path = entry.path.clone();
            let child_col = Column::new(child_path);
            self.columns.push(child_col);
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
        }
    }

    fn breadcrumb(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for col in &self.columns[..=self.active_col] {
            if let Some(name) = col.path.file_name() {
                parts.push(name.to_string_lossy().into_owned());
            } else {
                parts.push("/".to_string());
            }
        }
        parts.join("  ›  ")
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // Split: columns area on top, breadcrumb on bottom
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let col_area = chunks[0];
    let status_area = chunks[1];

    // Render breadcrumb
    let crumb = Paragraph::new(Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(app.breadcrumb(), Style::default().fg(Color::White)),
    ]))
    .style(Style::default().bg(Color::Rgb(30, 30, 30)));
    frame.render_widget(crumb, status_area);

    // Determine how many columns can fit (min 18 chars each)
    let num_cols = app.columns.len();
    // If we have many columns, only show last N that fit
    let visible_cols = (col_area.width / 20).max(1) as usize;
    let start_col = if num_cols > visible_cols {
        num_cols - visible_cols
    } else {
        0
    };

    let visible_count = num_cols - start_col;
    let constraints: Vec<Constraint> =
        (0..visible_count).map(|_| Constraint::Ratio(1, visible_count as u32)).collect();

    let col_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(col_area);

    for (vi, ci) in (start_col..num_cols).enumerate() {
        let col = &mut app.columns[ci];
        let is_active = ci == app.active_col;

        let border_style = if is_active {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::Rgb(60, 60, 60))
        };

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
            .border_style(border_style)
            .style(Style::default().bg(Color::Rgb(20, 20, 20)));

        let inner = block.inner(col_chunks[vi]);
        frame.render_widget(block, col_chunks[vi]);

        // Build list items
        let selected_path = col.selected_entry().map(|e| e.path.clone());
        let (items, _) = col.grouped.list_items(selected_path.as_deref());

        let highlight_style = if is_active {
            Style::default().bg(Color::Rgb(0, 92, 197)).fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default().bg(Color::Rgb(60, 60, 60)).fg(Color::White)
        };

        let list = List::new(items)
            .highlight_style(highlight_style)
            .style(Style::default().fg(Color::Rgb(200, 200, 200)));

        frame.render_stateful_widget(list, inner, &mut col.list_state);
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() -> io::Result<()> {
    let start = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap());

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(start);

    loop {
        terminal.draw(|f| render(f, &mut app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Up | KeyCode::Char('k') => app.move_up(),
                KeyCode::Down | KeyCode::Char('j') => app.move_down(),
                KeyCode::Right | KeyCode::Char('l') => app.move_right(),
                KeyCode::Left | KeyCode::Char('h') => app.move_left(),
                _ => {}
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    Ok(())
}
