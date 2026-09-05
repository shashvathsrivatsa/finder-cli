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

    pub fn scroll_by(&mut self, delta: isize, viewport_height: usize) {
        if self.grouped.row_count == 0 || viewport_height == 0 { return; }

        // Move offset
        let total_items = self.grouped.list_index_for_row(self.grouped.row_count - 1) + 1;
        let max_offset = total_items.saturating_sub(viewport_height);
        let new_offset = (self.list_state.offset() as isize + delta)
            .clamp(0, max_offset as isize) as usize;
        *self.list_state.offset_mut() = new_offset;

        // Cursor stays fixed; only move it if it falls outside the visible range
        let cursor_li = self.grouped.list_index_for_row(self.selected_row);
        if cursor_li < new_offset {
            // Scrolled past cursor at top — clamp cursor to first visible entry
            if let Some(row) = (new_offset..new_offset + viewport_height)
                .find_map(|li| self.grouped.entry_row_for_list_index(li))
            {
                self.selected_row = row;
            }
        } else if cursor_li >= new_offset + viewport_height {
            // Scrolled past cursor at bottom — clamp cursor to last visible entry
            let bottom = (new_offset + viewport_height).saturating_sub(1);
            if let Some(row) = (0..=bottom).rev()
                .find_map(|li| self.grouped.entry_row_for_list_index(li))
            {
                self.selected_row = row;
            }
        }

        self.list_state.select(Some(self.grouped.list_index_for_row(self.selected_row)));
    }
}
