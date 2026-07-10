use std::path::Path;

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{ListItem, ListState},
};

use crate::entry::{Entry, group_label, icon_for_entry};
use crate::rename::RenameState;

pub fn jump_label(row: usize) -> String {
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

#[derive(Debug)]
pub struct GroupedEntries {
    pub groups: Vec<(String, Vec<usize>)>,
    pub entries: Vec<Entry>,
    pub row_count: usize,
    pub row_to_entry: Vec<usize>,
}

impl GroupedEntries {
    pub fn build(entries: Vec<Entry>) -> Self {
        let mut folder_indices:   Vec<usize> = Vec::new();
        let mut exec_indices:     Vec<usize> = Vec::new();
        let mut dev_indices:      Vec<usize> = Vec::new();
        let mut config_indices:   Vec<usize> = Vec::new();
        let mut script_indices:   Vec<usize> = Vec::new();
        let mut compiled_indices: Vec<usize> = Vec::new();
        let mut image_indices:    Vec<usize> = Vec::new();
        let mut video_indices:    Vec<usize> = Vec::new();
        let mut audio_indices:    Vec<usize> = Vec::new();
        let mut doc_indices:      Vec<usize> = Vec::new();
        let mut font_indices:     Vec<usize> = Vec::new();
        let mut security_indices: Vec<usize> = Vec::new();
        let mut other_indices:    Vec<usize> = Vec::new();

        for (i, e) in entries.iter().enumerate() {
            if e.is_dir {
                folder_indices.push(i);
            } else if e.is_executable {
                exec_indices.push(i);
            } else {
                let ext = e.path.extension().and_then(|s| s.to_str()).unwrap_or("");
                match group_label(ext) {
                    "Developer" => dev_indices.push(i),
                    "Config"    => config_indices.push(i),
                    "Scripts"   => script_indices.push(i),
                    "Compiled"  => compiled_indices.push(i),
                    "Images"    => image_indices.push(i),
                    "Video"     => video_indices.push(i),
                    "Audio"     => audio_indices.push(i),
                    "Documents" => doc_indices.push(i),
                    "Fonts"     => font_indices.push(i),
                    "Security"  => security_indices.push(i),
                    _           => other_indices.push(i),
                }
            }
        }

        let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
        for (label, idxs) in [
            ("Folders",     folder_indices),
            ("Executables", exec_indices),
            ("Developer",   dev_indices),
            ("Config",      config_indices),
            ("Scripts",     script_indices),
            ("Compiled",    compiled_indices),
            ("Images",      image_indices),
            ("Video",       video_indices),
            ("Audio",       audio_indices),
            ("Documents",   doc_indices),
            ("Fonts",       font_indices),
            ("Security",    security_indices),
            ("Other",       other_indices),
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

    pub fn list_items(&self, selected_entry_path: Option<&Path>, renaming: Option<&RenameState>) -> (Vec<ListItem<'static>>, usize) {
        let mut items: Vec<ListItem<'static>> = Vec::new();
        let mut selected_item_index: usize = 0;
        let label_width = if self.row_count <= 10 { 1usize } else { 2 };
        let mut row_idx = 0usize;

        for (group_idx, (label, idxs)) in self.groups.iter().enumerate() {
            if group_idx > 0 {
                items.push(ListItem::new(Line::from("")));
            }
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
                let is_selected = selected_entry_path.is_some_and(|p| p == e.path);
                let label_str = format!("{:>width$} ", jump_label(row_idx), width = label_width);

                let mut spans = vec![
                    Span::styled(label_str, Style::default().fg(Color::Rgb(80, 80, 80))),
                ];

                if is_selected && renaming.is_some() {
                    spans.extend(renaming.unwrap().name_spans(icon, icon_color));
                } else {
                    let entry_label = if e.is_dir {
                        format!("{}/", e.name)
                    } else {
                        e.name.clone()
                    };
                    spans.push(Span::styled(format!("{} ", icon), Style::default().fg(icon_color)));
                    spans.push(Span::raw(entry_label));
                }

                if is_selected {
                    selected_item_index = items.len();
                }

                items.push(ListItem::new(Line::from(spans)));
                row_idx += 1;
            }
        }

        (items, selected_item_index)
    }

    pub fn entry_at_row(&self, row: usize) -> Option<&Entry> {
        self.row_to_entry.get(row).map(|&i| &self.entries[i])
    }

    pub fn list_index_for_row(&self, row: usize) -> usize {
        let mut list_idx = 0usize;
        let mut remaining = row;
        for (gi, (_, idxs)) in self.groups.iter().enumerate() {
            if gi > 0 { list_idx += 1; }
            list_idx += 1;
            if remaining < idxs.len() {
                return list_idx + remaining;
            }
            remaining -= idxs.len();
            list_idx += idxs.len();
        }
        list_idx
    }

    #[allow(dead_code)]
    pub fn entry_row_for_list_index(&self, list_idx: usize) -> Option<usize> {
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

    pub fn list_state_for_row(&self, row: usize) -> ListState {
        let mut state = ListState::default();
        if self.row_count > 0 {
            state.select(Some(self.list_index_for_row(row)));
        }
        state
    }
}
