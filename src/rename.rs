use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

#[derive(Clone, Debug, PartialEq)]
pub enum RenameMode {
    Normal,
    Insert,
}

pub struct RenameState {
    pub text: String,
    pub cursor: usize,
    pub mode: RenameMode,
    pub pending: String, // accumulates multi-key sequences: "d", "c", "di", "ci", "r"
}

impl RenameState {
    pub fn new(name: &str) -> Self {
        let cursor = name.rfind('.').unwrap_or(name.len());
        let cursor = if cursor == 0 { name.len() } else { cursor };
        RenameState { text: name.to_string(), cursor, mode: RenameMode::Insert, pending: String::new() }
    }

    pub fn char_count(&self) -> usize { self.text.chars().count() }

    fn byte_pos(&self, char_idx: usize) -> usize {
        self.text.char_indices().nth(char_idx).map(|(i, _)| i).unwrap_or(self.text.len())
    }

    fn clamp(&mut self) {
        if self.mode == RenameMode::Normal {
            let max = self.char_count().saturating_sub(1);
            if self.cursor > max { self.cursor = max; }
        }
    }

    fn is_word(c: char) -> bool { c.is_alphanumeric() || c == '_' }

    fn word_start_at(&self, pos: usize) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        if pos == 0 || chars.is_empty() { return 0; }
        let mut p = pos.min(chars.len().saturating_sub(1));
        if Self::is_word(chars[p]) {
            while p > 0 && Self::is_word(chars[p - 1]) { p -= 1; }
        } else {
            while p > 0 && !Self::is_word(chars[p - 1]) { p -= 1; }
        }
        p
    }

    fn word_end_at(&self, pos: usize) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        if chars.is_empty() { return 0; }
        let mut p = pos.min(chars.len().saturating_sub(1));
        if Self::is_word(chars[p]) {
            while p + 1 < chars.len() && Self::is_word(chars[p + 1]) { p += 1; }
        } else {
            while p + 1 < chars.len() && !Self::is_word(chars[p + 1]) { p += 1; }
        }
        p
    }

    // ── movement ──────────────────────────────────────────────────────────────

    pub fn move_left(&mut self) { if self.cursor > 0 { self.cursor -= 1; } }

    pub fn move_right(&mut self) {
        let max = if self.mode == RenameMode::Normal { self.char_count().saturating_sub(1) } else { self.char_count() };
        if self.cursor < max { self.cursor += 1; }
    }

    pub fn move_line_start(&mut self) { self.cursor = 0; }

    pub fn move_line_end(&mut self) {
        self.cursor = if self.mode == RenameMode::Normal { self.char_count().saturating_sub(1) } else { self.char_count() };
    }

    pub fn move_word_forward(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let len = chars.len();
        if self.cursor >= len { return; }
        if Self::is_word(chars[self.cursor]) {
            while self.cursor < len && Self::is_word(chars[self.cursor]) { self.cursor += 1; }
        } else {
            while self.cursor < len && !Self::is_word(chars[self.cursor]) { self.cursor += 1; }
        }
        self.clamp();
    }

    pub fn move_word_backward(&mut self) {
        if self.cursor == 0 { return; }
        let chars: Vec<char> = self.text.chars().collect();
        self.cursor -= 1;
        while self.cursor > 0 && chars[self.cursor] == ' ' { self.cursor -= 1; }
        if Self::is_word(chars[self.cursor]) {
            while self.cursor > 0 && Self::is_word(chars[self.cursor - 1]) { self.cursor -= 1; }
        } else {
            while self.cursor > 0 && !Self::is_word(chars[self.cursor - 1]) { self.cursor -= 1; }
        }
    }

    pub fn move_word_end(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let len = chars.len();
        if self.cursor + 1 >= len { return; }
        self.cursor += 1;
        while self.cursor < len && chars[self.cursor] == ' ' { self.cursor += 1; }
        if self.cursor < len {
            if Self::is_word(chars[self.cursor]) {
                while self.cursor + 1 < len && Self::is_word(chars[self.cursor + 1]) { self.cursor += 1; }
            } else {
                while self.cursor + 1 < len && !Self::is_word(chars[self.cursor + 1]) { self.cursor += 1; }
            }
        }
        self.clamp();
    }

    // ── insert / delete primitives ────────────────────────────────────────────

    pub fn insert_char(&mut self, c: char) {
        let bp = self.byte_pos(self.cursor);
        self.text.insert(bp, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let bp = self.byte_pos(self.cursor);
            self.text.remove(bp);
        }
    }

    pub fn delete_at_cursor(&mut self) {
        if self.cursor < self.char_count() {
            let bp = self.byte_pos(self.cursor);
            self.text.remove(bp);
            self.clamp();
        }
    }

    pub fn replace_char(&mut self, c: char) {
        if self.cursor < self.char_count() {
            let bp = self.byte_pos(self.cursor);
            self.text.remove(bp);
            self.text.insert(bp, c);
        }
    }

    pub fn clear_text(&mut self) { self.text.clear(); self.cursor = 0; }

    // ── range deletions ───────────────────────────────────────────────────────

    /// Delete chars [from, to] inclusive.
    fn delete_range(&mut self, from: usize, to: usize) {
        if from > to { return; }
        let chars: Vec<char> = self.text.chars().collect();
        let end = (to + 1).min(chars.len());
        self.text = chars[..from].iter().chain(chars[end..].iter()).collect();
        self.cursor = from;
        self.clamp();
    }

    /// de / ce — delete to end of current word (inclusive)
    pub fn delete_to_word_end(&mut self) {
        let end = self.word_end_at(self.cursor);
        self.delete_range(self.cursor, end);
    }

    /// dw — delete to start of next word (including trailing whitespace)
    pub fn delete_word_forward(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let len = chars.len();
        let mut end = self.cursor;
        if end < len {
            if Self::is_word(chars[end]) {
                while end < len && Self::is_word(chars[end]) { end += 1; }
            } else {
                while end < len && !Self::is_word(chars[end]) { end += 1; }
            }
            while end < len && chars[end] == ' ' { end += 1; }
        }
        if end > self.cursor {
            self.text = chars[..self.cursor].iter().chain(chars[end..].iter()).collect();
            self.clamp();
        }
    }

    /// db / cb — delete to start of current word
    pub fn delete_to_word_start(&mut self) {
        let start = self.word_start_at(self.cursor);
        if start == self.cursor { return; }
        let chars: Vec<char> = self.text.chars().collect();
        self.text = chars[..start].iter().chain(chars[self.cursor..].iter()).collect();
        self.cursor = start;
        self.clamp();
    }

    /// d$ / D / c$ / C — delete from cursor to end of text
    pub fn delete_to_line_end(&mut self) {
        self.text = self.text.chars().take(self.cursor).collect();
        self.clamp();
    }

    /// d0 / c0 — delete from start of text to cursor
    pub fn delete_to_line_start(&mut self) {
        self.text = self.text.chars().skip(self.cursor).collect();
        self.cursor = 0;
    }

    /// diw / ciw — delete the word under the cursor
    pub fn delete_inner_word(&mut self) {
        let start = self.word_start_at(self.cursor);
        let end = self.word_end_at(self.cursor);
        self.delete_range(start, end);
    }

    // ── mode transitions ──────────────────────────────────────────────────────

    pub fn enter_insert_before(&mut self) { self.mode = RenameMode::Insert; }
    pub fn enter_insert_after(&mut self) { if self.cursor < self.char_count() { self.cursor += 1; } self.mode = RenameMode::Insert; }
    pub fn enter_insert_start(&mut self) { self.cursor = 0; self.mode = RenameMode::Insert; }
    pub fn enter_insert_end(&mut self) { self.cursor = self.char_count(); self.mode = RenameMode::Insert; }

    pub fn enter_normal(&mut self) {
        self.mode = RenameMode::Normal;
        if self.cursor > 0 { self.cursor -= 1; }
        self.clamp();
    }

    // ── rendering ─────────────────────────────────────────────────────────────

    pub fn name_spans(&self, icon: &'static str, icon_color: Color) -> Vec<Span<'static>> {
        let chars: Vec<char> = self.text.chars().collect();
        let before: String = chars[..self.cursor].iter().collect();
        let icon_span = Span::styled(format!("{} ", icon), Style::default().fg(icon_color));

        match self.mode {
            RenameMode::Insert => {
                let after: String = chars[self.cursor..].iter().collect();
                vec![
                    icon_span,
                    Span::raw(before),
                    Span::styled("\u{2502}", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::raw(after),
                ]
            }
            RenameMode::Normal => {
                if self.cursor < chars.len() {
                    let cur: String = std::iter::once(chars[self.cursor]).collect();
                    let after: String = chars[self.cursor + 1..].iter().collect();
                    vec![
                        icon_span,
                        Span::raw(before),
                        Span::styled(cur, Style::default().bg(Color::White).fg(Color::Black)),
                        Span::raw(after),
                    ]
                } else {
                    vec![
                        icon_span,
                        Span::raw(before),
                        Span::styled(" ", Style::default().bg(Color::White).fg(Color::Black)),
                    ]
                }
            }
        }
    }
}
