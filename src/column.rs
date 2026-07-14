use std::path::PathBuf;

use ratatui::widgets::ListState;

use crate::entry::{Entry, read_dir_entries};
use crate::grouped::GroupedEntries;

pub struct Column {
    pub path: PathBuf,
    pub grouped: GroupedEntries,
    pub selected_row: usize,
    pub list_state: ListState,
}

impl Column {
    pub fn new(path: PathBuf) -> Self {
        let entries = read_dir_entries(&path);
        let grouped = GroupedEntries::build(entries);
        let list_state = grouped.list_state_for_row(0);
        Self { path, grouped, selected_row: 0, list_state }
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        self.grouped.entry_at_row(self.selected_row)
    }

    pub fn move_up(&mut self) {
        if self.selected_row > 0 {
            self.selected_row -= 1;
        }
        self.sync_list_state();
    }

    pub fn move_down(&mut self) {
        if self.grouped.row_count > 0 && self.selected_row + 1 < self.grouped.row_count {
            self.selected_row += 1;
        }
        self.sync_list_state();
    }

    pub fn move_by(&mut self, delta: isize) {
        let max = self.grouped.row_count.saturating_sub(1);
        self.selected_row = (self.selected_row as isize + delta).clamp(0, max as isize) as usize;
        self.sync_list_state();
    }

    pub fn sync_list_state(&mut self) {
        if self.grouped.row_count > 0 {
            let li = self.grouped.list_index_for_row(self.selected_row);
            self.list_state.select(Some(li));
        }
    }
}
