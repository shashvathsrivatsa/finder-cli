use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::{AtomicI64, AtomicU64}};
use std::time::Instant;

use crate::column::Column;
use crate::entry::read_dir_entries;
use crate::grouped::GroupedEntries;
use crate::rename::RenameState;


// Tweak this to change how long the "cut/copy: filename" flash shows
pub const CLIPBOARD_FLASH_MS: u64 = 200;

// Idle delay before preview size loads (ms)
pub const PREVIEW_DELAY_MS: u128 = 0;

// How many rows Ctrl+D / Ctrl+U jump (half-page feel)
pub const PAGE_JUMP: usize = 10;

#[derive(Clone, PartialEq)]
pub enum ClipboardOp { Cut, Copy }

#[derive(Clone)]
pub struct ClipboardEntry {
    pub op: ClipboardOp,
    pub path: PathBuf,      // primary path (for single-item ops and flash display)
    pub paths: Vec<PathBuf>, // all paths (single-item: vec![path])
    pub set_at: Instant,
}

#[derive(Clone)]
pub struct PaneInfo {
    pub id: String,
    pub label: String,
    pub same_session: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum PreviewMode { Long, Short, Name }

#[derive(Clone)]
pub struct ConvertState {
    pub source: PathBuf,
    pub formats: Vec<&'static str>,
    pub selected: usize,
}

pub struct App {
    pub columns: Vec<Column>,
    pub active_col: usize,
    pub pending_g: bool,
    pub pending_prefix: Option<usize>,
    pub cd_target: Option<PathBuf>,
    pub renaming: Option<RenameState>,
    pub confirming_delete: Option<PathBuf>,   // display name for confirmation
    pub pending_deletes: Vec<PathBuf>,         // all paths to delete on confirm
    pub is_deleting: bool,
    pub is_pasting: bool,
    pub is_converting: bool,
    pub convert_output: Option<PathBuf>,
    pub spinner_frame: usize,
    pub bg_done_rx: Option<std::sync::mpsc::Receiver<()>>,
    pub bg_progress: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
    pub bg_total: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
    pub last_key_at: Instant,
    pub preview_path: Option<PathBuf>,           // path the preview was computed for
    pub preview_size: Option<Arc<AtomicU64>>,   // u64::MAX = still computing, else bytes
    pub preview_modified: Option<Arc<AtomicI64>>, // i64::MIN = computing, else unix secs
    pub preview_created: Option<Arc<AtomicI64>>,  // i64::MIN = computing, -1 = unavailable, else unix secs
    pub preview_count: Option<Arc<AtomicI64>>,    // i64::MIN = computing, -1 = not a dir, else count
    pub preview_dims: Option<Arc<AtomicI64>>,     // i64::MIN = computing, -1 = n/a, else (w<<32)|h
    pub preview_fps: Option<Arc<AtomicI64>>,      // i64::MIN = computing, -1 = n/a, else fps*100
    pub preview_duration: Option<Arc<AtomicI64>>, // i64::MIN = computing, -1 = n/a, else seconds
    pub preview_pages: Option<Arc<AtomicI64>>,    // i64::MIN = computing, -1 = n/a, else page count
    pub clipboard: Option<ClipboardEntry>,
    pub focused: bool,
    pub linked_pane: Option<PaneInfo>,
    pub pane_picker: Option<(Vec<PaneInfo>, usize)>, // (panes, selected_idx)
    pub select_mode: bool,
    pub pending_digits: usize,
    pub col_viewport_height: usize,
    pub goto_query: Option<String>,
    pub selection: HashSet<PathBuf>,
    pub selection_anchor: Option<usize>, // row of last space-toggled entry
    pub preview_mode: PreviewMode,
    pub favorites: HashSet<PathBuf>,
    pub favorites_view: bool,
    pub favorites_cursor: usize,
    pub goto_base_dir: Option<PathBuf>,
    pub converting: Option<ConvertState>,
    pub status_flash: Option<(String, std::time::Instant)>,
}

impl App {
    pub fn new(start: PathBuf) -> Self {
        let col = Column::new(start);
        let mut app = App {
            columns: vec![col],
            active_col: 0,
            pending_g: false,
            pending_prefix: None,
            cd_target: None,
            renaming: None,
            confirming_delete: None,
            pending_deletes: Vec::new(),
            is_deleting: false,
            is_pasting: false,
            is_converting: false,
            convert_output: None,
            spinner_frame: 0,
            bg_done_rx: None,
            bg_progress: None,
            bg_total: None,
            last_key_at: Instant::now(),
            preview_path: None,
            preview_size: None,
            preview_modified: None,
            preview_created: None,
            preview_count: None,
            preview_dims: None,
            preview_fps: None,
            preview_duration: None,
            preview_pages: None,
            clipboard: None,
            focused: true,
            linked_pane: None,
            pane_picker: None,
            select_mode: false,
            pending_digits: 0,
            col_viewport_height: 0,
            goto_query: None,
            selection: HashSet::new(),
            selection_anchor: None,
            preview_mode: PreviewMode::Short,
            favorites: load_favorites(),
            favorites_view: false,
            favorites_cursor: 0,
            goto_base_dir: None,
            converting: None,
            status_flash: None,
        };
        app.maybe_push_child_column();
        app
    }

    pub fn maybe_push_child_column(&mut self) {
        let selected_dir = self.columns[self.active_col]
            .selected_entry()
            .filter(|e| e.is_dir)
            .map(|e| e.path.clone());

        match selected_dir {
            None => {
                self.columns.truncate(self.active_col + 1);
            }
            Some(path) => {
                if self.columns.get(self.active_col + 1).is_some_and(|c| c.path == path) {
                    return;
                }
                self.columns.truncate(self.active_col + 1);
                self.columns.push(Column::new(path));
            }
        }
    }

    pub fn refresh(&mut self) {
        for col in &mut self.columns {
            let new_entries = read_dir_entries(&col.path);
            let new_grouped = GroupedEntries::build(new_entries);
            let old_name = col.grouped.entry_at_row(col.selected_row).map(|e| e.name.clone());
            col.grouped = new_grouped;
            col.selected_row = old_name
                .and_then(|name| {
                    col.grouped
                        .row_to_entry
                        .iter()
                        .position(|&i| col.grouped.entries[i].name == name)
                })
                .unwrap_or_else(|| col.selected_row.min(col.grouped.row_count.saturating_sub(1)));
            col.sync_list_state();
        }
    }

    pub fn move_up(&mut self) {
        self.columns[self.active_col].move_up();
        self.maybe_push_child_column();
    }

    pub fn move_down(&mut self) {
        self.columns[self.active_col].move_down();
        self.maybe_push_child_column();
    }

    pub fn move_right(&mut self) {
        let can_move = self.columns
            .get(self.active_col)
            .and_then(|c| c.selected_entry())
            .is_some_and(|e| e.is_dir)
            && self.columns.len() > self.active_col + 1;
        if can_move {
            self.active_col += 1;
            self.maybe_push_child_column();
        }
    }

    pub fn move_left(&mut self) {
        if self.active_col > 0 {
            self.active_col -= 1;
        } else {
            let current_path = self.columns[0].path.clone();
            if let Some(parent) = current_path.parent() {
                let mut parent_col = Column::new(parent.to_path_buf());
                if let Some(row) = parent_col.grouped.row_to_entry.iter().position(|&i| {
                    parent_col.grouped.entries[i].path == current_path
                }) {
                    parent_col.selected_row = row;
                    parent_col.sync_list_state();
                }
                self.columns.insert(0, parent_col);
            }
        }
    }
}

pub fn convert_formats_for(path: &Path) -> Vec<&'static str> {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => vec!["png", "webp", "tiff", "pdf"],
        "png"          => vec!["jpg", "webp", "tiff", "pdf"],
        "webp"         => vec!["jpg", "png", "tiff", "pdf"],
        "tiff" | "tif" => vec!["jpg", "png", "webp", "pdf"],
        "heic" | "heif"=> vec!["jpg", "png", "webp", "tiff", "pdf"],
        "gif"          => vec!["mp4", "webm", "png"],
        "mp4"          => vec!["mov", "webm", "gif", "mp3"],
        "mov"          => vec!["mp4", "webm", "gif", "mp3"],
        "webm"         => vec!["mp4", "mov", "gif", "mp3"],
        "avi" | "mkv"  => vec!["mp4", "mov", "webm", "mp3"],
        "mp3"          => vec!["wav", "flac", "ogg", "aac"],
        "wav"          => vec!["mp3", "flac", "ogg", "aac"],
        "flac"         => vec!["mp3", "wav", "ogg", "aac"],
        "ogg" | "aac"  => vec!["mp3", "wav", "flac"],
        "doc" | "docx" | "odt" | "rtf" | "txt" | "md" | "mdx" => vec!["pdf"],
        "xls" | "xlsx" | "ods" | "csv" => vec!["pdf"],
        "ppt" | "pptx" | "odp"         => vec!["pdf"],
        _              => vec![],
    }
}

pub fn unique_output_path(source: &Path, new_ext: &str) -> PathBuf {
    let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    let dir = source.parent().unwrap_or(Path::new("."));
    let mut candidate = dir.join(format!("{}.{}", stem, new_ext));
    let mut stem_s = stem.to_string();
    while candidate.exists() {
        stem_s = format!("{} copy", stem_s);
        candidate = dir.join(format!("{}.{}", stem_s, new_ext));
    }
    candidate
}

fn favorites_path() -> PathBuf {
    let base = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&base).join(".local/share/dir-viewer/favorites")
}

pub fn load_favorites() -> HashSet<PathBuf> {
    let path = favorites_path();
    std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect()
}

pub fn save_favorites(favorites: &HashSet<PathBuf>) {
    let path = favorites_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let content: String = favorites.iter()
        .filter_map(|p| p.to_str())
        .map(|s| format!("{}\n", s))
        .collect();
    let _ = std::fs::write(&path, content);
}
