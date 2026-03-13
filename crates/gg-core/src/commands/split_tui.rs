//! TUI for interactive hunk selection in `gg split`
//!
//! A two-panel terminal UI (ratatui + crossterm) for selecting hunks:
//! - Left panel: File list with [✓]/[~]/[ ] indicators and hunk counts
//! - Right panel: Colored diff hunks for the selected file

use std::io;

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};

use super::split::DiffHunk;
use crate::error::{GgError, Result};

/// Result of the TUI hunk selection + inline commit message
#[derive(Debug, Clone)]
pub struct TuiResult {
    /// Indices of selected hunks
    pub selected_indices: Vec<usize>,
    /// Commit message entered inline
    pub commit_message: String,
    /// Remainder commit message entered inline (None if --no-edit was used)
    pub remainder_message: Option<String>,
}

/// Which panel is currently active
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panel {
    Files,
    Diff,
}

/// TUI interaction mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TuiMode {
    /// Selecting hunks (the main mode)
    HunkSelection,
    /// Typing the commit message inline (for the new commit)
    MessageInput,
    /// Typing the remainder commit message inline
    RemainderInput,
}

/// A file in the TUI file list
#[derive(Debug, Clone)]
struct TuiFile {
    /// File path
    path: String,
    /// Hunks for this file (indices into the original hunks vec)
    hunk_indices: Vec<usize>,
}

/// State for the split TUI
struct SplitTuiState {
    /// All hunks (original list)
    hunks: Vec<DiffHunk>,
    /// Selection state for each hunk (by index)
    selected: Vec<bool>,
    /// Files grouped from hunks
    files: Vec<TuiFile>,
    /// Currently active panel
    active_panel: Panel,
    /// Current TUI mode
    mode: TuiMode,
    /// Selected file index
    file_cursor: usize,
    /// Selected hunk index within the current file's hunks in the diff panel
    hunk_cursor: usize,
    /// Vertical scroll offset in the diff panel
    diff_scroll: usize,
    /// Whether user confirmed selection
    confirmed: bool,
    /// Whether user aborted
    aborted: bool,
    /// Commit message being edited (in MessageInput mode)
    message_text: String,
    /// Cursor position within the message text (byte offset)
    message_cursor: usize,
    /// Default commit message (pre-filled)
    default_message: String,
    /// Remainder commit message being edited (in RemainderInput mode)
    remainder_text: String,
    /// Cursor position within the remainder message text (byte offset)
    remainder_cursor: usize,
    /// Original commit message (used to pre-fill remainder)
    original_message: String,
    /// Whether to skip remainder message input (--no-edit)
    no_edit: bool,
}

impl SplitTuiState {
    fn new(
        hunks: Vec<DiffHunk>,
        default_message: String,
        original_message: String,
        no_edit: bool,
    ) -> Self {
        let selected = vec![false; hunks.len()];

        // Group hunks by file
        let mut files: Vec<TuiFile> = Vec::new();
        for (idx, hunk) in hunks.iter().enumerate() {
            if let Some(file) = files.iter_mut().find(|f| f.path == hunk.file_path) {
                file.hunk_indices.push(idx);
            } else {
                files.push(TuiFile {
                    path: hunk.file_path.clone(),
                    hunk_indices: vec![idx],
                });
            }
        }

        let message_cursor = default_message.len();
        let remainder_cursor = original_message.len();
        Self {
            hunks,
            selected,
            files,
            active_panel: Panel::Files,
            mode: TuiMode::HunkSelection,
            file_cursor: 0,
            hunk_cursor: 0,
            diff_scroll: 0,
            confirmed: false,
            aborted: false,
            message_text: default_message.clone(),
            message_cursor,
            default_message,
            remainder_text: original_message.clone(),
            remainder_cursor,
            original_message,
            no_edit,
        }
    }

    /// Get the current file (if any)
    fn current_file(&self) -> Option<&TuiFile> {
        self.files.get(self.file_cursor)
    }

    /// Get the hunk indices for the current file
    fn current_file_hunk_indices(&self) -> Vec<usize> {
        self.current_file()
            .map(|f| f.hunk_indices.clone())
            .unwrap_or_default()
    }

    /// Count selected hunks for a file
    fn selected_count_for_file(&self, file_idx: usize) -> (usize, usize) {
        if let Some(file) = self.files.get(file_idx) {
            let total = file.hunk_indices.len();
            let selected = file
                .hunk_indices
                .iter()
                .filter(|&&idx| self.selected[idx])
                .count();
            (selected, total)
        } else {
            (0, 0)
        }
    }

    /// Get total selection count
    fn total_selected(&self) -> usize {
        self.selected.iter().filter(|&&s| s).count()
    }

    /// Toggle selection based on active panel
    fn toggle_selection(&mut self) {
        match self.active_panel {
            Panel::Files => {
                // Toggle all hunks for this file
                if let Some(file) = self.files.get(self.file_cursor) {
                    let all_selected = file.hunk_indices.iter().all(|&idx| self.selected[idx]);
                    let new_state = !all_selected;
                    for &idx in &file.hunk_indices {
                        self.selected[idx] = new_state;
                    }
                }
            }
            Panel::Diff => {
                // Toggle the current hunk
                if let Some(file) = self.current_file() {
                    if let Some(&hunk_idx) = file.hunk_indices.get(self.hunk_cursor) {
                        self.selected[hunk_idx] = !self.selected[hunk_idx];
                    }
                }
            }
        }
    }

    /// Navigate up
    fn move_up(&mut self) {
        match self.active_panel {
            Panel::Files => {
                if self.file_cursor > 0 {
                    self.file_cursor -= 1;
                    self.hunk_cursor = 0;
                    self.diff_scroll = 0;
                }
            }
            Panel::Diff => {
                if self.hunk_cursor > 0 {
                    self.hunk_cursor -= 1;
                    // Adjust scroll if needed (handled in draw)
                }
            }
        }
    }

    /// Navigate down
    fn move_down(&mut self) {
        match self.active_panel {
            Panel::Files => {
                if self.file_cursor + 1 < self.files.len() {
                    self.file_cursor += 1;
                    self.hunk_cursor = 0;
                    self.diff_scroll = 0;
                }
            }
            Panel::Diff => {
                let hunk_count = self.current_file_hunk_indices().len();
                if self.hunk_cursor + 1 < hunk_count {
                    self.hunk_cursor += 1;
                }
            }
        }
    }

    /// Switch to the other panel
    fn switch_panel(&mut self) {
        self.active_panel = match self.active_panel {
            Panel::Files => Panel::Diff,
            Panel::Diff => Panel::Files,
        };
    }

    /// Select all (scoped to panel context)
    fn select_all(&mut self) {
        match self.active_panel {
            Panel::Files => {
                // Select all hunks globally
                for s in &mut self.selected {
                    *s = true;
                }
            }
            Panel::Diff => {
                // Select all hunks for current file
                if let Some(file) = self.files.get(self.file_cursor) {
                    for &idx in &file.hunk_indices {
                        self.selected[idx] = true;
                    }
                }
            }
        }
    }

    /// Deselect all (scoped to panel context)
    fn deselect_all(&mut self) {
        match self.active_panel {
            Panel::Files => {
                // Deselect all hunks globally
                for s in &mut self.selected {
                    *s = false;
                }
            }
            Panel::Diff => {
                // Deselect all hunks for current file
                if let Some(file) = self.files.get(self.file_cursor) {
                    for &idx in &file.hunk_indices {
                        self.selected[idx] = false;
                    }
                }
            }
        }
    }

    /// Try to split the current hunk (only in diff panel)
    fn try_split_current_hunk(&mut self) -> bool {
        if self.active_panel != Panel::Diff {
            return false;
        }

        let file = match self.files.get(self.file_cursor) {
            Some(f) => f.clone(),
            None => return false,
        };

        let hunk_idx = match file.hunk_indices.get(self.hunk_cursor) {
            Some(&idx) => idx,
            None => return false,
        };

        let hunk = &self.hunks[hunk_idx];
        if let Some(sub_hunks) = super::split::try_split_hunk(hunk) {
            let num_sub_hunks = sub_hunks.len();

            // Get the current selection state
            let was_selected = self.selected[hunk_idx];

            // Remove the old hunk and insert sub-hunks
            self.hunks.splice(hunk_idx..=hunk_idx, sub_hunks);

            // Update selection state: insert new selection entries for sub-hunks
            self.selected.splice(
                hunk_idx..=hunk_idx,
                std::iter::repeat_n(was_selected, num_sub_hunks),
            );

            // Update all file hunk indices
            for file in &mut self.files {
                for idx in &mut file.hunk_indices {
                    if *idx > hunk_idx {
                        *idx += num_sub_hunks - 1;
                    }
                }
            }

            // Insert the new indices for sub-hunks into the current file
            if let Some(file) = self.files.get_mut(self.file_cursor) {
                // Find position of hunk_idx in hunk_indices
                if let Some(pos) = file.hunk_indices.iter().position(|&x| x == hunk_idx) {
                    // Remove old index and insert new ones
                    file.hunk_indices.remove(pos);
                    for i in 0..num_sub_hunks {
                        file.hunk_indices.insert(pos + i, hunk_idx + i);
                    }
                }
            }

            true
        } else {
            false
        }
    }

    /// Adjust diff_scroll so the current hunk cursor stays visible.
    /// Call this before drawing. `visible_height` is the inner height of the diff panel.
    fn adjust_diff_scroll(&mut self, visible_height: u16) {
        if self.active_panel != Panel::Diff {
            return;
        }

        let Some(file) = self.files.get(self.file_cursor) else {
            return;
        };

        // Calculate the line offset of the current hunk within the rendered diff.
        // Layout: 3 header lines (--- a/..., +++ b/..., blank), then per hunk:
        //   1 header line + N diff lines + 1 blank separator
        let mut line_offset: usize = 3; // file header lines
        for (i, &hunk_idx) in file.hunk_indices.iter().enumerate() {
            if i == self.hunk_cursor {
                break;
            }
            if let Some(hunk) = self.hunks.get(hunk_idx) {
                line_offset += 1 + hunk.lines.len() + 1; // header + lines + blank
            }
        }

        let vh = visible_height as usize;
        if vh == 0 {
            return;
        }

        // Scroll down if cursor is below visible area
        if line_offset >= self.diff_scroll + vh {
            self.diff_scroll = line_offset.saturating_sub(vh) + 1;
        }
        // Scroll up if cursor is above visible area
        if line_offset < self.diff_scroll {
            self.diff_scroll = line_offset;
        }
    }

    /// Get selected hunk indices (for return value)
    fn get_selected_indices(&self) -> Vec<usize> {
        self.selected
            .iter()
            .enumerate()
            .filter_map(|(idx, &sel)| if sel { Some(idx) } else { None })
            .collect()
    }

    /// Enter message input mode (called when user confirms hunk selection)
    fn enter_message_mode(&mut self) {
        self.mode = TuiMode::MessageInput;
        // Reset message to default and place cursor at end
        self.message_text = self.default_message.clone();
        self.message_cursor = self.message_text.len();
    }

    /// Return to hunk selection mode (called when user presses Esc in message input)
    fn exit_message_mode(&mut self) {
        self.mode = TuiMode::HunkSelection;
    }

    /// Enter remainder message input mode (called after new commit message is confirmed)
    fn enter_remainder_mode(&mut self) {
        self.mode = TuiMode::RemainderInput;
        // Reset remainder to original message and place cursor at end
        self.remainder_text = self.original_message.clone();
        self.remainder_cursor = self.remainder_text.len();
    }

    /// Return to message input mode (called when user presses Esc in remainder input)
    fn exit_remainder_mode(&mut self) {
        self.mode = TuiMode::MessageInput;
    }

    /// Get mutable references to the active text and cursor based on current mode
    fn active_text_and_cursor(&mut self) -> (&mut String, &mut usize) {
        match self.mode {
            TuiMode::RemainderInput => (&mut self.remainder_text, &mut self.remainder_cursor),
            _ => (&mut self.message_text, &mut self.message_cursor),
        }
    }

    /// Insert a character at the current cursor position
    fn message_insert_char(&mut self, c: char) {
        let (text, cursor) = self.active_text_and_cursor();
        text.insert(*cursor, c);
        *cursor += c.len_utf8();
    }

    /// Delete the character before the cursor (backspace)
    fn message_backspace(&mut self) {
        let (text, cursor) = self.active_text_and_cursor();
        if *cursor > 0 {
            let prev = text[..*cursor]
                .char_indices()
                .next_back()
                .map(|(idx, _)| idx)
                .unwrap_or(0);
            text.remove(prev);
            *cursor = prev;
        }
    }

    /// Delete the character at the cursor (delete key)
    fn message_delete(&mut self) {
        let (text, cursor) = self.active_text_and_cursor();
        if *cursor < text.len() {
            text.remove(*cursor);
        }
    }

    /// Move cursor left by one character
    fn message_cursor_left(&mut self) {
        let (text, cursor) = self.active_text_and_cursor();
        if *cursor > 0 {
            let prev = text[..*cursor]
                .char_indices()
                .next_back()
                .map(|(idx, _)| idx)
                .unwrap_or(0);
            *cursor = prev;
        }
    }

    /// Move cursor right by one character
    fn message_cursor_right(&mut self) {
        let (text, cursor) = self.active_text_and_cursor();
        if *cursor < text.len() {
            let next = text[*cursor..]
                .char_indices()
                .nth(1)
                .map(|(idx, _)| *cursor + idx)
                .unwrap_or(text.len());
            *cursor = next;
        }
    }

    /// Move cursor to beginning of message
    fn message_cursor_home(&mut self) {
        let (_text, cursor) = self.active_text_and_cursor();
        *cursor = 0;
    }

    /// Move cursor to end of message
    fn message_cursor_end(&mut self) {
        let (text, cursor) = self.active_text_and_cursor();
        *cursor = text.len();
    }
}

/// Terminal cleanup guard - restores terminal on drop
struct TerminalGuard {
    _private: (),
}

impl TerminalGuard {
    fn new() -> Result<Self> {
        enable_raw_mode()
            .map_err(|e| GgError::Other(format!("Failed to enable raw mode: {}", e)))?;
        if let Err(e) = execute!(io::stdout(), EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(GgError::Other(format!(
                "Failed to enter alternate screen: {}",
                e
            )));
        }
        Ok(Self { _private: () })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

/// Run the TUI for hunk selection with inline commit message input.
///
/// Returns `Ok(Some(TuiResult))` with selected hunk indices and commit message,
/// or `Ok(None)` if the user aborted.
///
/// - `commit_title` is the original commit's title, used to pre-fill the new message as
///   `"Split from: <title>"`.
/// - `original_message` is the full original commit message (without GG-ID), used to
///   pre-fill the remainder message input.
/// - `no_edit` — if true, skip the remainder message input and use original message as-is.
pub fn select_hunks_tui(
    hunks: Vec<DiffHunk>,
    commit_title: &str,
    original_message: &str,
    no_edit: bool,
) -> Result<Option<TuiResult>> {
    if hunks.is_empty() {
        return Err(GgError::Other("No hunks to display".to_string()));
    }

    let default_message = format!("Split from: {}", commit_title);

    // Set up terminal with cleanup guard
    let _guard = TerminalGuard::new()?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)
        .map_err(|e| GgError::Other(format!("Failed to create terminal: {}", e)))?;

    let mut state = SplitTuiState::new(
        hunks,
        default_message,
        original_message.to_string(),
        no_edit,
    );

    // Main loop
    loop {
        // Calculate visible height of the diff panel for scroll adjustment.
        // Main layout: content area (Min(1)) + status bar (Length(3)).
        // Content area: 33% files + 67% diff. Diff panel inner height = height - 2 (borders).
        let term_height = terminal.size().map(|s| s.height).unwrap_or(24);
        let content_height = term_height.saturating_sub(3);
        let diff_inner_height = content_height.saturating_sub(2);
        state.adjust_diff_scroll(diff_inner_height);

        terminal
            .draw(|f| draw(f, &state))
            .map_err(|e| GgError::Other(format!("Failed to draw: {}", e)))?;

        // Poll for events with timeout
        if event::poll(std::time::Duration::from_millis(100))
            .map_err(|e| GgError::Other(format!("Event poll failed: {}", e)))?
        {
            if let Event::Key(key) =
                event::read().map_err(|e| GgError::Other(format!("Event read failed: {}", e)))?
            {
                handle_key(&mut state, key.code, key.modifiers);

                if state.confirmed {
                    return Ok(Some(TuiResult {
                        selected_indices: state.get_selected_indices(),
                        commit_message: state.message_text.trim().to_string(),
                        remainder_message: if state.no_edit {
                            None
                        } else {
                            Some(state.remainder_text.trim().to_string())
                        },
                    }));
                }
                if state.aborted {
                    return Ok(None);
                }
            }
        }
    }
}

/// Handle a key press
fn handle_key(state: &mut SplitTuiState, code: KeyCode, modifiers: KeyModifiers) {
    // Handle Ctrl+C in raw mode (no SIGINT) — always aborts
    if modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char('c') = code {
            state.aborted = true;
            return;
        }
    }

    match state.mode {
        TuiMode::HunkSelection => handle_key_hunk_selection(state, code, modifiers),
        TuiMode::MessageInput => handle_key_message_input(state, code, modifiers),
        TuiMode::RemainderInput => handle_key_remainder_input(state, code, modifiers),
    }
}

/// Handle keys in hunk selection mode
fn handle_key_hunk_selection(state: &mut SplitTuiState, code: KeyCode, _modifiers: KeyModifiers) {
    match code {
        KeyCode::Up | KeyCode::Char('k') => state.move_up(),
        KeyCode::Down | KeyCode::Char('j') => state.move_down(),
        KeyCode::Char(' ') => state.toggle_selection(),
        KeyCode::Tab | KeyCode::Left | KeyCode::Right => state.switch_panel(),
        KeyCode::Char('a') => state.select_all(),
        KeyCode::Char('n') => state.deselect_all(),
        KeyCode::Char('s') => {
            state.try_split_current_hunk();
        }
        KeyCode::Enter => {
            // Switch to message input mode instead of confirming immediately
            state.enter_message_mode();
        }
        KeyCode::Char('q') | KeyCode::Esc => {
            state.aborted = true;
        }
        _ => {}
    }
}

/// Handle keys in message input mode
fn handle_key_message_input(state: &mut SplitTuiState, code: KeyCode, modifiers: KeyModifiers) {
    match code {
        KeyCode::Enter => {
            // Confirm with the entered message
            if state.message_text.trim().is_empty() {
                // Don't allow empty messages — stay in input mode
                return;
            }
            if state.no_edit {
                // Skip remainder input, confirm immediately
                state.confirmed = true;
            } else {
                // Move to remainder message input
                state.enter_remainder_mode();
            }
        }
        KeyCode::Esc => {
            // Go back to hunk selection
            state.exit_message_mode();
        }
        KeyCode::Backspace => state.message_backspace(),
        KeyCode::Delete => state.message_delete(),
        KeyCode::Left => state.message_cursor_left(),
        KeyCode::Right => state.message_cursor_right(),
        KeyCode::Home => state.message_cursor_home(),
        KeyCode::End => state.message_cursor_end(),
        KeyCode::Char(c) => {
            // Ctrl+A = home, Ctrl+E = end (emacs-style)
            if modifiers.contains(KeyModifiers::CONTROL) {
                match c {
                    'a' => state.message_cursor_home(),
                    'e' => state.message_cursor_end(),
                    'u' => {
                        // Ctrl+U: clear from cursor to beginning
                        let (text, cursor) = state.active_text_and_cursor();
                        *text = text[*cursor..].to_string();
                        *cursor = 0;
                    }
                    'k' => {
                        // Ctrl+K: clear from cursor to end
                        let (text, cursor) = state.active_text_and_cursor();
                        text.truncate(*cursor);
                    }
                    _ => {}
                }
            } else {
                state.message_insert_char(c);
            }
        }
        _ => {}
    }
}

/// Handle keys in remainder message input mode
fn handle_key_remainder_input(state: &mut SplitTuiState, code: KeyCode, modifiers: KeyModifiers) {
    match code {
        KeyCode::Enter => {
            // Confirm with the entered remainder message
            if state.remainder_text.trim().is_empty() {
                // Don't allow empty messages — stay in input mode
                return;
            }
            state.confirmed = true;
        }
        KeyCode::Esc => {
            // Go back to message input (not all the way to hunk selection)
            state.exit_remainder_mode();
        }
        KeyCode::Backspace => state.message_backspace(),
        KeyCode::Delete => state.message_delete(),
        KeyCode::Left => state.message_cursor_left(),
        KeyCode::Right => state.message_cursor_right(),
        KeyCode::Home => state.message_cursor_home(),
        KeyCode::End => state.message_cursor_end(),
        KeyCode::Char(c) => {
            if modifiers.contains(KeyModifiers::CONTROL) {
                match c {
                    'a' => state.message_cursor_home(),
                    'e' => state.message_cursor_end(),
                    'u' => {
                        // Ctrl+U: clear from cursor to beginning
                        let (text, cursor) = state.active_text_and_cursor();
                        *text = text[*cursor..].to_string();
                        *cursor = 0;
                    }
                    'k' => {
                        // Ctrl+K: clear from cursor to end
                        let (text, cursor) = state.active_text_and_cursor();
                        text.truncate(*cursor);
                    }
                    _ => {}
                }
            } else {
                state.message_insert_char(c);
            }
        }
        _ => {}
    }
}

/// Draw the TUI
fn draw(f: &mut Frame, state: &SplitTuiState) {
    let size = f.area();

    // Bottom bar height depends on mode
    let bottom_height = match state.mode {
        TuiMode::HunkSelection => 3,
        TuiMode::MessageInput => 3,
        TuiMode::RemainderInput => 3,
    };

    // Main layout: content area + status/input bar
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(bottom_height)])
        .split(size);

    // Content area: file panel (1/3) + diff panel (2/3)
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(33), Constraint::Percentage(67)])
        .split(main_chunks[0]);

    draw_file_panel(f, state, content_chunks[0]);
    draw_diff_panel(f, state, content_chunks[1]);

    match state.mode {
        TuiMode::HunkSelection => draw_status_bar(f, state, main_chunks[1]),
        TuiMode::MessageInput => draw_message_input(f, state, main_chunks[1]),
        TuiMode::RemainderInput => draw_remainder_input(f, state, main_chunks[1]),
    }
}

/// Draw the file panel (left side)
fn draw_file_panel(f: &mut Frame, state: &SplitTuiState, area: Rect) {
    let is_active = state.active_panel == Panel::Files;

    let items: Vec<ListItem> = state
        .files
        .iter()
        .enumerate()
        .map(|(idx, file)| {
            let (selected, total) = state.selected_count_for_file(idx);

            // Determine checkbox style
            let (checkbox, checkbox_style) = if selected == total && total > 0 {
                ("[✓]", Style::default().fg(Color::Green))
            } else if selected > 0 {
                ("[~]", Style::default().fg(Color::Yellow))
            } else {
                ("[ ]", Style::default().fg(Color::DarkGray))
            };

            // Truncate path if needed (char-boundary-aware for UTF-8 safety)
            let max_path_len = area.width.saturating_sub(12) as usize;
            let display_path = if max_path_len == 0 {
                String::new()
            } else {
                let chars: Vec<char> = file.path.chars().collect();
                if chars.len() > max_path_len {
                    let start_char = chars.len() - max_path_len + 1;
                    format!("…{}", chars[start_char..].iter().collect::<String>())
                } else {
                    file.path.clone()
                }
            };

            let line = Line::from(vec![
                Span::styled(checkbox, checkbox_style),
                Span::raw(" "),
                Span::raw(display_path),
                Span::styled(
                    format!(" ({})", total),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);

            let mut item = ListItem::new(line);

            // Highlight current item
            if idx == state.file_cursor {
                let style = if is_active {
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().bg(Color::DarkGray).fg(Color::White)
                };
                item = item.style(style);
            }

            item
        })
        .collect();

    let border_style = if is_active {
        Style::default().fg(Color::Blue)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Files ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

/// Draw the diff panel (right side)
fn draw_diff_panel(f: &mut Frame, state: &SplitTuiState, area: Rect) {
    let is_active = state.active_panel == Panel::Diff;

    let border_style = if is_active {
        Style::default().fg(Color::Blue)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Diff ")
        .borders(Borders::ALL)
        .border_style(border_style);

    // Build diff lines
    let mut lines: Vec<Line> = Vec::new();

    if let Some(file) = state.current_file() {
        // File header
        lines.push(Line::from(vec![Span::styled(
            format!("--- a/{}", file.path),
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!("+++ b/{}", file.path),
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));

        for (hunk_list_idx, &hunk_idx) in file.hunk_indices.iter().enumerate() {
            if let Some(hunk) = state.hunks.get(hunk_idx) {
                let is_selected = state.selected[hunk_idx];
                let is_current =
                    state.active_panel == Panel::Diff && hunk_list_idx == state.hunk_cursor;

                // Checkbox for hunk
                let (checkbox, checkbox_style) = if is_selected {
                    ("[✓]", Style::default().fg(Color::Green))
                } else {
                    ("[ ]", Style::default().fg(Color::DarkGray))
                };

                // Hunk header with checkbox
                let mut header_line = vec![
                    Span::styled(checkbox, checkbox_style),
                    Span::raw(" "),
                    Span::styled(&hunk.header, Style::default().fg(Color::Cyan)),
                ];

                // Add highlight marker for current hunk
                if is_current {
                    header_line.insert(0, Span::styled("▶ ", Style::default().fg(Color::Yellow)));
                } else {
                    header_line.insert(0, Span::raw("  "));
                }

                lines.push(Line::from(header_line));

                // Hunk lines
                for diff_line in &hunk.lines {
                    let line_str = format!(
                        "    {}{}",
                        diff_line.origin,
                        diff_line.content.trim_end_matches('\n')
                    );
                    let style = match diff_line.origin {
                        '+' => Style::default().fg(Color::Green),
                        '-' => Style::default().fg(Color::Red),
                        _ => Style::default().fg(Color::DarkGray),
                    };
                    lines.push(Line::from(Span::styled(line_str, style)));
                }

                lines.push(Line::from("")); // Blank line between hunks
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            "No file selected",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((state.diff_scroll as u16, 0));

    f.render_widget(paragraph, area);
}

/// Draw the status bar (bottom, in HunkSelection mode)
fn draw_status_bar(f: &mut Frame, state: &SplitTuiState, area: Rect) {
    let selected = state.total_selected();
    let total = state.hunks.len();

    let status_text = format!(
        " {}/{} hunks selected │ [Space] toggle · [Tab] switch panel · [a]ll · [n]one · [s]plit · [Enter] confirm · [q]uit ",
        selected, total
    );

    let paragraph = Paragraph::new(status_text)
        .style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, area);
}

/// Draw the inline commit message input (bottom, in MessageInput mode)
fn draw_message_input(f: &mut Frame, state: &SplitTuiState, area: Rect) {
    let label = " New commit message: ";

    // Build spans: label + text before cursor + cursor char + text after cursor
    let (before, cursor_char, after) = {
        let text = &state.message_text;
        let pos = state.message_cursor;
        let before = &text[..pos];
        if pos < text.len() {
            let ch = text[pos..].chars().next().unwrap();
            let ch_len = ch.len_utf8();
            let after = &text[pos + ch_len..];
            (before.to_string(), ch.to_string(), after.to_string())
        } else {
            (before.to_string(), " ".to_string(), String::new())
        }
    };

    let line = Line::from(vec![
        Span::styled(
            label,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(&before),
        Span::styled(
            &cursor_char,
            Style::default().bg(Color::White).fg(Color::Black),
        ),
        Span::raw(&after),
        Span::styled(
            "  [Enter] confirm · [Esc] back",
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let paragraph = Paragraph::new(line)
        .style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, area);
}

/// Draw the inline remainder commit message input (bottom, in RemainderInput mode)
fn draw_remainder_input(f: &mut Frame, state: &SplitTuiState, area: Rect) {
    let label = " Remainder commit message: ";

    // Build spans: label + text before cursor + cursor char + text after cursor
    let (before, cursor_char, after) = {
        let text = &state.remainder_text;
        let pos = state.remainder_cursor;
        let before = &text[..pos];
        if pos < text.len() {
            let ch = text[pos..].chars().next().unwrap();
            let ch_len = ch.len_utf8();
            let after = &text[pos + ch_len..];
            (before.to_string(), ch.to_string(), after.to_string())
        } else {
            (before.to_string(), " ".to_string(), String::new())
        }
    };

    let line = Line::from(vec![
        Span::styled(
            label,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(&before),
        Span::styled(
            &cursor_char,
            Style::default().bg(Color::White).fg(Color::Black),
        ),
        Span::raw(&after),
        Span::styled(
            "  [Enter] confirm · [Esc] back",
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let paragraph = Paragraph::new(line)
        .style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::super::split::DiffLine;
    use super::*;

    fn make_test_hunk(file: &str, header: &str) -> DiffHunk {
        DiffHunk {
            file_path: file.to_string(),
            header: header.to_string(),
            lines: vec![
                DiffLine {
                    origin: ' ',
                    content: "context line\n".to_string(),
                },
                DiffLine {
                    origin: '+',
                    content: "added line\n".to_string(),
                },
                DiffLine {
                    origin: '-',
                    content: "removed line\n".to_string(),
                },
            ],
            old_start: 1,
            old_lines: 2,
            new_start: 1,
            new_lines: 2,
        }
    }

    #[test]
    fn test_state_initialization() {
        let hunks = vec![
            make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@"),
            make_test_hunk("file1.rs", "@@ -10,2 +10,2 @@"),
            make_test_hunk("file2.rs", "@@ -1,2 +1,2 @@"),
        ];

        let state = SplitTuiState::new(
            hunks,
            "Split from: test commit".to_string(),
            "test commit".to_string(),
            false,
        );

        assert_eq!(state.files.len(), 2);
        assert_eq!(state.files[0].path, "file1.rs");
        assert_eq!(state.files[0].hunk_indices, vec![0, 1]);
        assert_eq!(state.files[1].path, "file2.rs");
        assert_eq!(state.files[1].hunk_indices, vec![2]);
        assert_eq!(state.selected, vec![false, false, false]);
        assert_eq!(state.active_panel, Panel::Files);
    }

    #[test]
    fn test_toggle_file() {
        let hunks = vec![
            make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@"),
            make_test_hunk("file1.rs", "@@ -10,2 +10,2 @@"),
            make_test_hunk("file2.rs", "@@ -1,2 +1,2 @@"),
        ];

        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test commit".to_string(),
            "test commit".to_string(),
            false,
        );

        // Toggle file1 (should select both hunks)
        state.toggle_selection();
        assert_eq!(state.selected, vec![true, true, false]);

        // Toggle again (should deselect both)
        state.toggle_selection();
        assert_eq!(state.selected, vec![false, false, false]);
    }

    #[test]
    fn test_toggle_hunk_in_diff_panel() {
        let hunks = vec![
            make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@"),
            make_test_hunk("file1.rs", "@@ -10,2 +10,2 @@"),
        ];

        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test commit".to_string(),
            "test commit".to_string(),
            false,
        );
        state.active_panel = Panel::Diff;

        // Toggle first hunk
        state.toggle_selection();
        assert_eq!(state.selected, vec![true, false]);

        // Move down and toggle second hunk
        state.move_down();
        state.toggle_selection();
        assert_eq!(state.selected, vec![true, true]);
    }

    #[test]
    fn test_navigation() {
        let hunks = vec![
            make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@"),
            make_test_hunk("file2.rs", "@@ -1,2 +1,2 @@"),
            make_test_hunk("file3.rs", "@@ -1,2 +1,2 @@"),
        ];

        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test commit".to_string(),
            "test commit".to_string(),
            false,
        );

        assert_eq!(state.file_cursor, 0);

        state.move_down();
        assert_eq!(state.file_cursor, 1);

        state.move_down();
        assert_eq!(state.file_cursor, 2);

        // Can't go past the end
        state.move_down();
        assert_eq!(state.file_cursor, 2);

        state.move_up();
        assert_eq!(state.file_cursor, 1);

        state.move_up();
        assert_eq!(state.file_cursor, 0);

        // Can't go before the start
        state.move_up();
        assert_eq!(state.file_cursor, 0);
    }

    #[test]
    fn test_select_all_deselect_all() {
        let hunks = vec![
            make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@"),
            make_test_hunk("file1.rs", "@@ -10,2 +10,2 @@"),
            make_test_hunk("file2.rs", "@@ -1,2 +1,2 @@"),
        ];

        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test commit".to_string(),
            "test commit".to_string(),
            false,
        );

        // Select all globally (in Files panel)
        state.select_all();
        assert_eq!(state.selected, vec![true, true, true]);

        // Deselect all globally
        state.deselect_all();
        assert_eq!(state.selected, vec![false, false, false]);

        // Switch to diff panel and select all for current file only
        state.active_panel = Panel::Diff;
        state.select_all();
        assert_eq!(state.selected, vec![true, true, false]); // Only file1

        // Deselect all for current file
        state.deselect_all();
        assert_eq!(state.selected, vec![false, false, false]);
    }

    #[test]
    fn test_panel_switching() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test commit".to_string(),
            "test commit".to_string(),
            false,
        );

        assert_eq!(state.active_panel, Panel::Files);

        state.switch_panel();
        assert_eq!(state.active_panel, Panel::Diff);

        state.switch_panel();
        assert_eq!(state.active_panel, Panel::Files);
    }

    #[test]
    fn test_selected_count_for_file() {
        let hunks = vec![
            make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@"),
            make_test_hunk("file1.rs", "@@ -10,2 +10,2 @@"),
            make_test_hunk("file2.rs", "@@ -1,2 +1,2 @@"),
        ];

        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test commit".to_string(),
            "test commit".to_string(),
            false,
        );

        // Initial: nothing selected
        assert_eq!(state.selected_count_for_file(0), (0, 2));
        assert_eq!(state.selected_count_for_file(1), (0, 1));

        // Select first hunk of file1
        state.selected[0] = true;
        assert_eq!(state.selected_count_for_file(0), (1, 2));

        // Select both hunks of file1
        state.selected[1] = true;
        assert_eq!(state.selected_count_for_file(0), (2, 2));
    }

    #[test]
    fn test_get_selected_indices() {
        let hunks = vec![
            make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@"),
            make_test_hunk("file1.rs", "@@ -10,2 +10,2 @@"),
            make_test_hunk("file2.rs", "@@ -1,2 +1,2 @@"),
        ];

        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test commit".to_string(),
            "test commit".to_string(),
            false,
        );
        state.selected[0] = true;
        state.selected[2] = true;

        let indices = state.get_selected_indices();
        assert_eq!(indices, vec![0, 2]);
    }

    #[test]
    fn test_total_selected() {
        let hunks = vec![
            make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@"),
            make_test_hunk("file2.rs", "@@ -1,2 +1,2 @@"),
            make_test_hunk("file3.rs", "@@ -1,2 +1,2 @@"),
        ];

        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test commit".to_string(),
            "test commit".to_string(),
            false,
        );
        assert_eq!(state.total_selected(), 0);

        state.selected[1] = true;
        assert_eq!(state.total_selected(), 1);

        state.selected[0] = true;
        state.selected[2] = true;
        assert_eq!(state.total_selected(), 3);
    }

    #[test]
    fn test_ctrl_c_aborts() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test commit".to_string(),
            "test commit".to_string(),
            false,
        );

        assert!(!state.aborted);
        handle_key(&mut state, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(state.aborted);
    }

    #[test]
    fn test_ctrl_c_does_not_trigger_without_modifier() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test commit".to_string(),
            "test commit".to_string(),
            false,
        );

        // Plain 'c' should not abort
        handle_key(&mut state, KeyCode::Char('c'), KeyModifiers::NONE);
        assert!(!state.aborted);
    }

    #[test]
    fn test_diff_scroll_adjusts_when_cursor_moves_past_visible_area() {
        // Create many hunks for a single file so they exceed visible height
        let hunks: Vec<DiffHunk> = (0..20)
            .map(|i| make_test_hunk("file1.rs", &format!("@@ -{},2 +{},2 @@", i * 10, i * 10)))
            .collect();

        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test commit".to_string(),
            "test commit".to_string(),
            false,
        );
        state.active_panel = Panel::Diff;

        // Small visible height to force scrolling
        let visible_height = 10u16;

        // Move cursor down past visible area
        for _ in 0..15 {
            state.move_down();
            state.adjust_diff_scroll(visible_height);
        }

        // diff_scroll should be non-zero now
        assert!(
            state.diff_scroll > 0,
            "diff_scroll should be adjusted when cursor moves past visible area"
        );

        // Move cursor back up
        for _ in 0..15 {
            state.move_up();
            state.adjust_diff_scroll(visible_height);
        }

        // diff_scroll should return to 3 (first hunk starts after 3 header lines)
        assert_eq!(
            state.diff_scroll, 3,
            "diff_scroll should return to first hunk position when cursor is at top"
        );
    }

    #[test]
    fn test_diff_scroll_zero_height() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test commit".to_string(),
            "test commit".to_string(),
            false,
        );
        state.active_panel = Panel::Diff;

        // Should not panic with zero visible height
        state.adjust_diff_scroll(0);
        assert_eq!(state.diff_scroll, 0);
    }

    #[test]
    fn test_path_truncation_narrow_terminal() {
        // Simulate the truncation logic with width <= 12 (max_path_len = 0)
        let path = "src/very/long/path/to/file.rs";
        let width: u16 = 10; // narrower than 12
        let max_path_len = width.saturating_sub(12) as usize;
        let display_path = if max_path_len == 0 {
            String::new()
        } else {
            let chars: Vec<char> = path.chars().collect();
            if chars.len() > max_path_len {
                let start_char = chars.len() - max_path_len + 1;
                format!("…{}", chars[start_char..].iter().collect::<String>())
            } else {
                path.to_string()
            }
        };
        assert_eq!(display_path, "");
    }

    #[test]
    fn test_path_truncation_non_ascii() {
        // Non-ASCII path should not panic
        let path = "src/日本語/ファイル.rs";
        let width: u16 = 25;
        let max_path_len = width.saturating_sub(12) as usize; // 13
        let display_path = if max_path_len == 0 {
            String::new()
        } else {
            let chars: Vec<char> = path.chars().collect();
            if chars.len() > max_path_len {
                let start_char = chars.len() - max_path_len + 1;
                format!("…{}", chars[start_char..].iter().collect::<String>())
            } else {
                path.to_string()
            }
        };
        // 'src/日本語/ファイル.rs' is 18 chars, max 13 → truncated
        assert!(display_path.starts_with('…'));
        assert!(display_path.len() <= path.len());
    }

    // ====================================================================
    // Message input mode tests
    // ====================================================================

    #[test]
    fn test_enter_message_mode() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test commit".to_string(),
            "test commit".to_string(),
            false,
        );

        assert_eq!(state.mode, TuiMode::HunkSelection);

        state.enter_message_mode();
        assert_eq!(state.mode, TuiMode::MessageInput);
        assert_eq!(state.message_text, "Split from: test commit");
        assert_eq!(state.message_cursor, "Split from: test commit".len());
    }

    #[test]
    fn test_exit_message_mode() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test commit".to_string(),
            "test commit".to_string(),
            false,
        );

        state.enter_message_mode();
        assert_eq!(state.mode, TuiMode::MessageInput);

        state.exit_message_mode();
        assert_eq!(state.mode, TuiMode::HunkSelection);
    }

    #[test]
    fn test_enter_in_hunk_selection_switches_to_message_input() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test commit".to_string(),
            "test commit".to_string(),
            false,
        );

        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(state.mode, TuiMode::MessageInput);
        assert!(!state.confirmed);
    }

    #[test]
    fn test_esc_in_message_input_returns_to_hunk_selection() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test commit".to_string(),
            "test commit".to_string(),
            false,
        );

        state.enter_message_mode();
        handle_key(&mut state, KeyCode::Esc, KeyModifiers::NONE);

        assert_eq!(state.mode, TuiMode::HunkSelection);
        assert!(!state.aborted);
    }

    #[test]
    fn test_enter_in_message_input_confirms() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test commit".to_string(),
            "test commit".to_string(),
            false,
        );

        state.enter_message_mode();
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);

        // Enter in MessageInput now goes to RemainderInput (no_edit=false)
        assert_eq!(state.mode, TuiMode::RemainderInput);
        assert!(!state.confirmed);

        // Enter in RemainderInput confirms
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert!(state.confirmed);
        assert_eq!(state.message_text, "Split from: test commit");
    }

    #[test]
    fn test_enter_with_empty_message_does_not_confirm() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(hunks, String::new(), "test commit".to_string(), false);

        state.enter_message_mode();
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);

        assert!(!state.confirmed);
        assert_eq!(state.mode, TuiMode::MessageInput);
    }

    #[test]
    fn test_message_insert_char() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(hunks, String::new(), "test commit".to_string(), false);

        state.enter_message_mode();
        state.message_insert_char('H');
        state.message_insert_char('i');

        assert_eq!(state.message_text, "Hi");
        assert_eq!(state.message_cursor, 2);
    }

    #[test]
    fn test_message_backspace() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state =
            SplitTuiState::new(hunks, "Hello".to_string(), "test commit".to_string(), false);

        state.enter_message_mode();
        assert_eq!(state.message_cursor, 5);

        state.message_backspace();
        assert_eq!(state.message_text, "Hell");
        assert_eq!(state.message_cursor, 4);

        // Backspace at position 0 does nothing
        state.message_cursor_home();
        state.message_backspace();
        assert_eq!(state.message_text, "Hell");
        assert_eq!(state.message_cursor, 0);
    }

    #[test]
    fn test_message_delete() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state =
            SplitTuiState::new(hunks, "Hello".to_string(), "test commit".to_string(), false);

        state.enter_message_mode();
        state.message_cursor_home();

        state.message_delete();
        assert_eq!(state.message_text, "ello");
        assert_eq!(state.message_cursor, 0);

        // Delete at end does nothing
        state.message_cursor_end();
        state.message_delete();
        assert_eq!(state.message_text, "ello");
    }

    #[test]
    fn test_message_cursor_movement() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state =
            SplitTuiState::new(hunks, "Hello".to_string(), "test commit".to_string(), false);

        state.enter_message_mode();
        assert_eq!(state.message_cursor, 5); // at end

        state.message_cursor_left();
        assert_eq!(state.message_cursor, 4);

        state.message_cursor_left();
        assert_eq!(state.message_cursor, 3);

        state.message_cursor_right();
        assert_eq!(state.message_cursor, 4);

        state.message_cursor_home();
        assert_eq!(state.message_cursor, 0);

        state.message_cursor_end();
        assert_eq!(state.message_cursor, 5);

        // Can't go past bounds
        state.message_cursor_right();
        assert_eq!(state.message_cursor, 5);

        state.message_cursor_home();
        state.message_cursor_left();
        assert_eq!(state.message_cursor, 0);
    }

    #[test]
    fn test_message_insert_at_middle() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state =
            SplitTuiState::new(hunks, "Helo".to_string(), "test commit".to_string(), false);

        state.enter_message_mode();
        // Move cursor to position 3 (before 'o')
        state.message_cursor = 3;
        state.message_insert_char('l');

        assert_eq!(state.message_text, "Hello");
        assert_eq!(state.message_cursor, 4);
    }

    #[test]
    fn test_message_ctrl_u_clears_to_beginning() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Hello World".to_string(),
            "test commit".to_string(),
            false,
        );

        state.enter_message_mode();
        state.message_cursor = 5; // after "Hello"

        handle_key(&mut state, KeyCode::Char('u'), KeyModifiers::CONTROL);

        assert_eq!(state.message_text, " World");
        assert_eq!(state.message_cursor, 0);
    }

    #[test]
    fn test_message_ctrl_k_clears_to_end() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Hello World".to_string(),
            "test commit".to_string(),
            false,
        );

        state.enter_message_mode();
        state.message_cursor = 5; // after "Hello"

        handle_key(&mut state, KeyCode::Char('k'), KeyModifiers::CONTROL);

        assert_eq!(state.message_text, "Hello");
        assert_eq!(state.message_cursor, 5);
    }

    #[test]
    fn test_ctrl_c_aborts_from_message_input() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state =
            SplitTuiState::new(hunks, "test".to_string(), "test commit".to_string(), false);

        state.enter_message_mode();
        handle_key(&mut state, KeyCode::Char('c'), KeyModifiers::CONTROL);

        assert!(state.aborted);
    }

    #[test]
    fn test_message_typing_via_handle_key() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(hunks, String::new(), "test commit".to_string(), false);

        state.enter_message_mode();

        // Type "Hi" using handle_key
        handle_key(&mut state, KeyCode::Char('H'), KeyModifiers::NONE);
        handle_key(&mut state, KeyCode::Char('i'), KeyModifiers::NONE);

        assert_eq!(state.message_text, "Hi");
        assert!(!state.confirmed);

        // Press Enter → goes to remainder input
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.mode, TuiMode::RemainderInput);
        assert!(!state.confirmed);

        // Press Enter in remainder to confirm
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert!(state.confirmed);
    }

    #[test]
    fn test_full_flow_hunk_selection_to_message_to_confirm() {
        let hunks = vec![
            make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@"),
            make_test_hunk("file2.rs", "@@ -1,2 +1,2 @@"),
        ];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: original".to_string(),
            "test commit".to_string(),
            false,
        );

        // Select a hunk
        handle_key(&mut state, KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(state.selected, vec![true, false]);

        // Press Enter → message input mode
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.mode, TuiMode::MessageInput);

        // Confirm new commit message → remainder input
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.mode, TuiMode::RemainderInput);
        assert!(!state.confirmed);

        // Confirm remainder message
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert!(state.confirmed);
        assert_eq!(state.message_text, "Split from: original");
    }

    #[test]
    fn test_full_flow_message_edit_and_go_back() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: original".to_string(),
            "test commit".to_string(),
            false,
        );

        // Enter message mode
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.mode, TuiMode::MessageInput);

        // Go back with Esc
        handle_key(&mut state, KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(state.mode, TuiMode::HunkSelection);
        assert!(!state.aborted);

        // Can still navigate hunks
        handle_key(&mut state, KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(state.selected, vec![true]);
    }

    #[test]
    fn test_message_unicode_handling() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(hunks, String::new(), "test commit".to_string(), false);

        state.enter_message_mode();
        state.message_insert_char('é');
        state.message_insert_char('ñ');

        assert_eq!(state.message_text, "éñ");
        assert_eq!(state.message_cursor, "éñ".len());

        state.message_backspace();
        assert_eq!(state.message_text, "é");

        state.message_cursor_left();
        assert_eq!(state.message_cursor, 0);
    }

    // ====================================================================
    // Remainder input mode tests
    // ====================================================================

    #[test]
    fn test_enter_remainder_mode() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test".to_string(),
            "original commit message".to_string(),
            false,
        );

        state.enter_message_mode();
        // Confirm new commit message → should go to remainder
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.mode, TuiMode::RemainderInput);
        assert_eq!(state.remainder_text, "original commit message");
        assert_eq!(state.remainder_cursor, "original commit message".len());
        assert!(!state.confirmed);
    }

    #[test]
    fn test_esc_from_remainder_goes_to_message_input() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test".to_string(),
            "original".to_string(),
            false,
        );

        state.enter_message_mode();
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE); // → RemainderInput
        assert_eq!(state.mode, TuiMode::RemainderInput);

        handle_key(&mut state, KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(state.mode, TuiMode::MessageInput);
        assert!(!state.aborted);
    }

    #[test]
    fn test_esc_navigation_remainder_to_message_to_hunk() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test".to_string(),
            "original".to_string(),
            false,
        );

        // HunkSelection → MessageInput → RemainderInput
        state.enter_message_mode();
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.mode, TuiMode::RemainderInput);

        // Esc → back to MessageInput
        handle_key(&mut state, KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(state.mode, TuiMode::MessageInput);

        // Esc → back to HunkSelection
        handle_key(&mut state, KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(state.mode, TuiMode::HunkSelection);
        assert!(!state.aborted);
    }

    #[test]
    fn test_no_edit_skips_remainder_input() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test".to_string(),
            "original".to_string(),
            true, // no_edit = true
        );

        state.enter_message_mode();
        // Enter in MessageInput should confirm directly (skip remainder)
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert!(state.confirmed);
        // Should NOT have entered RemainderInput
        assert_eq!(state.mode, TuiMode::MessageInput);
    }

    #[test]
    fn test_remainder_enter_confirms() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test".to_string(),
            "original message".to_string(),
            false,
        );

        state.enter_message_mode();
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE); // → RemainderInput
        assert_eq!(state.mode, TuiMode::RemainderInput);

        // Enter confirms from remainder
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert!(state.confirmed);
        assert_eq!(state.remainder_text, "original message");
    }

    #[test]
    fn test_remainder_empty_message_does_not_confirm() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state =
            SplitTuiState::new(hunks, "Split from: test".to_string(), String::new(), false);

        state.enter_message_mode();
        // Type something for new commit message so we can proceed
        handle_key(&mut state, KeyCode::Char('x'), KeyModifiers::NONE);
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE); // → RemainderInput

        // Remainder is empty, Enter should not confirm
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert!(!state.confirmed);
        assert_eq!(state.mode, TuiMode::RemainderInput);
    }

    #[test]
    fn test_remainder_typing_and_editing() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test".to_string(),
            "original".to_string(),
            false,
        );

        state.enter_message_mode();
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE); // → RemainderInput
        assert_eq!(state.mode, TuiMode::RemainderInput);

        // Clear and type new remainder message
        handle_key(&mut state, KeyCode::Char('u'), KeyModifiers::CONTROL); // Ctrl+U clears to beginning
                                                                           // Cursor is at end, so Ctrl+U clears everything before cursor
                                                                           // Actually Ctrl+U clears from cursor to beginning, so we need cursor at end first
                                                                           // The cursor is at end after entering remainder mode, so Ctrl+U clears all
        state.remainder_text.clear();
        state.remainder_cursor = 0;

        // Type new message
        handle_key(&mut state, KeyCode::Char('n'), KeyModifiers::NONE);
        handle_key(&mut state, KeyCode::Char('e'), KeyModifiers::NONE);
        handle_key(&mut state, KeyCode::Char('w'), KeyModifiers::NONE);

        assert_eq!(state.remainder_text, "new");
    }

    #[test]
    fn test_full_flow_with_both_messages() {
        let hunks = vec![
            make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@"),
            make_test_hunk("file2.rs", "@@ -1,2 +1,2 @@"),
        ];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: original".to_string(),
            "original commit msg".to_string(),
            false,
        );

        // Select a hunk
        handle_key(&mut state, KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(state.selected, vec![true, false]);

        // Enter → message input
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.mode, TuiMode::MessageInput);

        // Confirm new commit message → remainder input
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.mode, TuiMode::RemainderInput);
        assert_eq!(state.remainder_text, "original commit msg");

        // Confirm remainder message
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert!(state.confirmed);
        assert_eq!(state.message_text, "Split from: original");
        assert_eq!(state.remainder_text, "original commit msg");
    }

    #[test]
    fn test_ctrl_c_aborts_from_remainder_input() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test".to_string(),
            "original".to_string(),
            false,
        );

        state.enter_message_mode();
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE); // → RemainderInput
        handle_key(&mut state, KeyCode::Char('c'), KeyModifiers::CONTROL);

        assert!(state.aborted);
    }

    #[test]
    fn test_remainder_cursor_movement() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test".to_string(),
            "Hello".to_string(),
            false,
        );

        state.enter_remainder_mode();
        assert_eq!(state.remainder_cursor, 5); // at end

        state.message_cursor_left();
        assert_eq!(state.remainder_cursor, 4);

        state.message_cursor_home();
        assert_eq!(state.remainder_cursor, 0);

        state.message_cursor_end();
        assert_eq!(state.remainder_cursor, 5);
    }

    #[test]
    fn test_remainder_backspace() {
        let hunks = vec![make_test_hunk("file1.rs", "@@ -1,2 +1,2 @@")];
        let mut state = SplitTuiState::new(
            hunks,
            "Split from: test".to_string(),
            "Hello".to_string(),
            false,
        );

        state.enter_remainder_mode();
        state.message_backspace();
        assert_eq!(state.remainder_text, "Hell");
        assert_eq!(state.remainder_cursor, 4);
    }
}
