use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{ListItem, ListState},
};

use crate::entry::{Entry, group_label, icon_for_entry, icon_for_name};
use crate::rename::RenameState;

pub fn jump_label(row: usize, width: usize) -> String {
    format!("{:0>width$}", row + 1, width = width)
}

pub fn label_width(row_count: usize) -> usize {
    if row_count == 0 { 1 } else { row_count.to_string().len() }
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
        let mut network_indices:  Vec<usize> = Vec::new();
        let mut other_indices:    Vec<usize> = Vec::new();

        for (i, e) in entries.iter().enumerate() {
            if e.is_dir {
                folder_indices.push(i);
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
                    "Network"   => network_indices.push(i),
                    _ if e.is_executable => exec_indices.push(i),
                    _           => other_indices.push(i),
                }
            }
        }

        let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
        for (label, idxs) in [
            ("",            folder_indices),
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
            ("Network",     network_indices),
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

    pub fn list_items(&self, selected_entry_path: Option<&Path>, renaming: Option<&RenameState>, selection: &HashSet<PathBuf>) -> (Vec<ListItem<'static>>, usize) {
        let mut items: Vec<ListItem<'static>> = Vec::new();
        let mut selected_item_index: usize = 0;
        let lw = label_width(self.row_count);
        let mut row_idx = 0usize;

        for (group_idx, (label, idxs)) in self.groups.iter().enumerate() {
            if !label.is_empty() {
                if group_idx > 0 {
                    items.push(ListItem::new(Line::from("")));
                }
                let gutter = " ".repeat(lw + 1);
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
            } else if group_idx > 0 {
                items.push(ListItem::new(Line::from("")));
            }

            for &ei in idxs {
                let e = &self.entries[ei];
                let is_selected = selected_entry_path.is_some_and(|p| p == e.path);
                let (icon, icon_color) = if is_selected && renaming.is_some() && !e.is_dir {
                    icon_for_name(&renaming.unwrap().text)
                } else {
                    icon_for_entry(e)
                };
                let label_str = format!("{} ", jump_label(row_idx, lw));

                let x = 100;
                let mut spans = vec![
                    Span::styled(label_str, Style::default().fg(Color::Rgb(x, x, x))),
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

                let in_selection = selection.contains(&e.path);
                let item = ListItem::new(Line::from(spans));
                let item = if in_selection {
                    item.style(Style::default().bg(Color::Rgb(60, 40, 90)))
                } else {
                    item
                };
                items.push(item);
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
        for (gi, (label, idxs)) in self.groups.iter().enumerate() {
            if gi > 0 { list_idx += 1; } // spacer
            if !label.is_empty() { list_idx += 1; } // header
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
        for (gi, (label, idxs)) in self.groups.iter().enumerate() {
            if gi > 0 {
                if idx == list_idx { return None; }
                idx += 1;
            }
            if !label.is_empty() {
                if idx == list_idx { return None; }
                idx += 1;
            }
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
