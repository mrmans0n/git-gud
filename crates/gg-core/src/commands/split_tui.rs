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

/// Which panel is currently active
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panel {
    Files,
    Diff,
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
}

impl SplitTuiState {
    fn new(hunks: Vec<DiffHunk>) -> Self {
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

        Self {
            hunks,
            selected,
            files,
            active_panel: Panel::Files,
            file_cursor: 0,
            hunk_cursor: 0,
            diff_scroll: 0,
            confirmed: false,
            aborted: false,
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

    /// Get selected hunk indices (for return value)
    fn get_selected_indices(&self) -> Vec<usize> {
        self.selected
            .iter()
            .enumerate()
            .filter_map(|(idx, &sel)| if sel { Some(idx) } else { None })
            .collect()
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
        execute!(io::stdout(), EnterAlternateScreen)
            .map_err(|e| GgError::Other(format!("Failed to enter alternate screen: {}", e)))?;
        Ok(Self { _private: () })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

/// Run the TUI for hunk selection
/// Returns selected hunk indices
pub fn select_hunks_tui(hunks: Vec<DiffHunk>) -> Result<Vec<usize>> {
    if hunks.is_empty() {
        return Err(GgError::Other("No hunks to display".to_string()));
    }

    // Set up terminal with cleanup guard
    let _guard = TerminalGuard::new()?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)
        .map_err(|e| GgError::Other(format!("Failed to create terminal: {}", e)))?;

    let mut state = SplitTuiState::new(hunks);

    // Main loop
    loop {
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
                    return Ok(state.get_selected_indices());
                }
                if state.aborted {
                    return Err(GgError::Other("Selection aborted".to_string()));
                }
            }
        }
    }
}

/// Handle a key press
fn handle_key(state: &mut SplitTuiState, code: KeyCode, _modifiers: KeyModifiers) {
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
            state.confirmed = true;
        }
        KeyCode::Char('q') | KeyCode::Esc => {
            state.aborted = true;
        }
        _ => {}
    }
}

/// Draw the TUI
fn draw(f: &mut Frame, state: &SplitTuiState) {
    let size = f.area();

    // Main layout: content area + status bar
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(size);

    // Content area: file panel (1/3) + diff panel (2/3)
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(33), Constraint::Percentage(67)])
        .split(main_chunks[0]);

    draw_file_panel(f, state, content_chunks[0]);
    draw_diff_panel(f, state, content_chunks[1]);
    draw_status_bar(f, state, main_chunks[1]);
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

            // Truncate path if needed
            let max_path_len = area.width.saturating_sub(12) as usize;
            let display_path = if file.path.len() > max_path_len {
                format!("…{}", &file.path[file.path.len() - max_path_len + 1..])
            } else {
                file.path.clone()
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

/// Draw the status bar (bottom)
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

        let state = SplitTuiState::new(hunks);

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

        let mut state = SplitTuiState::new(hunks);

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

        let mut state = SplitTuiState::new(hunks);
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

        let mut state = SplitTuiState::new(hunks);

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

        let mut state = SplitTuiState::new(hunks);

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
        let mut state = SplitTuiState::new(hunks);

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

        let mut state = SplitTuiState::new(hunks);

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

        let mut state = SplitTuiState::new(hunks);
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

        let mut state = SplitTuiState::new(hunks);
        assert_eq!(state.total_selected(), 0);

        state.selected[1] = true;
        assert_eq!(state.total_selected(), 1);

        state.selected[0] = true;
        state.selected[2] = true;
        assert_eq!(state.total_selected(), 3);
    }
}
