use std::path::PathBuf;
use std::time::Instant;

use crate::column::Column;
use crate::entry::read_dir_entries;
use crate::grouped::GroupedEntries;
use crate::rename::RenameState;

pub fn qwerty_prefix_offset(c: char) -> Option<usize> {
    "poiuytrew".find(c).map(|i| (i + 1) * 10)
}

// Tweak this to change how long the "cut/copy: filename" flash shows
pub const CLIPBOARD_FLASH_MS: u64 = 200;

#[derive(Clone, PartialEq)]
pub enum ClipboardOp { Cut, Copy }

#[derive(Clone)]
pub struct ClipboardEntry {
    pub op: ClipboardOp,
    pub path: PathBuf,
    pub set_at: Instant,
}

#[derive(Clone)]
pub struct PaneInfo {
    pub id: String,
    pub label: String,
    pub same_session: bool,
}

pub struct App {
    pub columns: Vec<Column>,
    pub active_col: usize,
    pub pending_g: bool,
    pub pending_prefix: Option<usize>,
    pub cd_target: Option<PathBuf>,
    pub renaming: Option<RenameState>,
    pub confirming_delete: Option<PathBuf>,
    pub clipboard: Option<ClipboardEntry>,
    pub focused: bool,
    pub linked_pane: Option<PaneInfo>,
    pub pane_picker: Option<(Vec<PaneInfo>, usize)>, // (panes, selected_idx)
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
            clipboard: None,
            focused: true,
            linked_pane: None,
            pane_picker: None,
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
