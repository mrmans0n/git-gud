use std::io::{self, IsTerminal};
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};

use super::{bucket_label, sanitize_human_diagnostic, ActionBucket, InboxCandidate};

pub(super) enum InboxRowState<'a> {
    Refreshing,
    Bucket(ActionBucket),
    MergedHidden,
    ClosedHidden,
    RefreshFailed(&'a str),
}

pub(super) struct LiveInboxRenderer {
    multi_progress: MultiProgress,
    rows: Vec<ProgressBar>,
    candidates: Vec<InboxCandidate>,
    provider_label: String,
    counter: ProgressBar,
    completed: usize,
    cleared: bool,
}

impl LiveInboxRenderer {
    pub fn stderr_if_interactive(
        candidates: &[InboxCandidate],
        provider_label: &str,
    ) -> Option<Self> {
        live_rendering_enabled(io::stdout().is_terminal(), io::stderr().is_terminal()).then(|| {
            Self::with_draw_target(candidates, provider_label, ProgressDrawTarget::stderr())
        })
    }

    fn with_draw_target(
        candidates: &[InboxCandidate],
        provider_label: &str,
        draw_target: ProgressDrawTarget,
    ) -> Self {
        let multi_progress = MultiProgress::with_draw_target(draw_target);
        let row_style =
            ProgressStyle::with_template("{spinner:.green} {msg}").expect("valid inbox row style");
        let mut rows = Vec::with_capacity(candidates.len());

        for candidate in candidates {
            let row = ProgressBar::new_spinner();
            row.set_style(row_style.clone());
            let row = multi_progress.add(row);
            row.set_message(format_row_message(
                candidate,
                provider_label,
                InboxRowState::Refreshing,
            ));
            row.enable_steady_tick(Duration::from_millis(100));
            rows.push(row);
        }

        let counter = ProgressBar::new_spinner();
        counter
            .set_style(ProgressStyle::with_template("{msg}").expect("valid inbox counter style"));
        let counter = multi_progress.add(counter);
        counter.set_message(format!("refreshed 0/{}", candidates.len()));

        Self {
            multi_progress,
            rows,
            candidates: candidates.to_vec(),
            provider_label: provider_label.to_string(),
            counter,
            completed: 0,
            cleared: false,
        }
    }

    pub fn update(&mut self, discovery_index: usize, state: InboxRowState<'_>) {
        let Some((candidate, row)) = self
            .candidates
            .get(discovery_index)
            .zip(self.rows.get(discovery_index))
        else {
            return;
        };

        row.finish_with_message(format_row_message(candidate, &self.provider_label, state));
        self.completed += 1;
        self.counter.set_message(format!(
            "refreshed {}/{}",
            self.completed,
            self.candidates.len()
        ));
    }

    pub fn finish_and_clear(&mut self) {
        self.clear();
    }

    fn clear(&mut self) {
        if self.cleared {
            return;
        }

        for row in &self.rows {
            row.disable_steady_tick();
        }
        let _ = self.multi_progress.clear();
        self.multi_progress
            .set_draw_target(ProgressDrawTarget::hidden());
        self.cleared = true;
    }
}

impl Drop for LiveInboxRenderer {
    fn drop(&mut self) {
        self.clear();
    }
}

pub(super) fn live_rendering_enabled(stdout_is_terminal: bool, stderr_is_terminal: bool) -> bool {
    stdout_is_terminal && stderr_is_terminal
}

fn format_row_message(
    candidate: &InboxCandidate,
    provider_label: &str,
    state: InboxRowState<'_>,
) -> String {
    let number_prefix = if provider_label == "MR" { "!" } else { "#" };
    let status = match state {
        InboxRowState::Refreshing => "refreshing".to_string(),
        InboxRowState::Bucket(bucket) => bucket_label(bucket).to_string(),
        InboxRowState::MergedHidden => "merged (hidden; use --all)".to_string(),
        InboxRowState::ClosedHidden => "closed (hidden)".to_string(),
        InboxRowState::RefreshFailed(error) => {
            format!(
                "refresh failed: {}",
                sanitize_human_diagnostic(error)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        }
    };

    format!(
        "{} #{}  {}  {}  {} {}{} — {}",
        candidate.stack_name,
        candidate.position,
        candidate.short_sha,
        candidate.title,
        provider_label,
        number_prefix,
        candidate.pr_number,
        status
    )
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use indicatif::{ProgressDrawTarget, TermLike};

    use super::{format_row_message, live_rendering_enabled, InboxRowState, LiveInboxRenderer};
    use crate::commands::inbox::{ActionBucket, InboxCandidate};

    const CLEAR: &str = "<clear>";
    const FLUSH: &str = "<flush>";

    #[derive(Debug)]
    struct RecordingTerm {
        operations: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingTerm {
        fn record(&self, operation: impl Into<String>) {
            self.operations.lock().unwrap().push(operation.into());
        }
    }

    impl TermLike for RecordingTerm {
        fn width(&self) -> u16 {
            160
        }

        fn height(&self) -> u16 {
            20
        }

        fn move_cursor_up(&self, _n: usize) -> io::Result<()> {
            Ok(())
        }

        fn move_cursor_down(&self, _n: usize) -> io::Result<()> {
            Ok(())
        }

        fn move_cursor_right(&self, _n: usize) -> io::Result<()> {
            Ok(())
        }

        fn move_cursor_left(&self, _n: usize) -> io::Result<()> {
            Ok(())
        }

        fn write_line(&self, line: &str) -> io::Result<()> {
            self.record(format!("{line}\n"));
            Ok(())
        }

        fn write_str(&self, value: &str) -> io::Result<()> {
            self.record(value);
            Ok(())
        }

        fn clear_line(&self) -> io::Result<()> {
            self.record(CLEAR);
            Ok(())
        }

        fn flush(&self) -> io::Result<()> {
            self.record(FLUSH);
            Ok(())
        }
    }

    fn candidate(
        discovery_index: usize,
        stack_name: &str,
        short_sha: &str,
        title: &str,
    ) -> InboxCandidate {
        InboxCandidate {
            discovery_index,
            stack_name: stack_name.to_string(),
            position: discovery_index + 1,
            short_sha: short_sha.to_string(),
            title: title.to_string(),
            pr_number: discovery_index as u64 + 10,
            behind_base: None,
        }
    }

    fn rendered_frames(operations: &[String]) -> Vec<String> {
        let mut frames = Vec::new();
        let mut frame = String::new();

        for operation in operations {
            match operation.as_str() {
                FLUSH => {
                    frames.push(std::mem::take(&mut frame));
                }
                CLEAR => {}
                written => frame.push_str(written),
            }
        }

        frames
    }

    #[test]
    fn live_rendering_requires_both_output_streams_to_be_terminals() {
        assert!(live_rendering_enabled(true, true));
        assert!(!live_rendering_enabled(true, false));
        assert!(!live_rendering_enabled(false, true));
        assert!(!live_rendering_enabled(false, false));
    }

    #[test]
    fn row_messages_keep_candidate_identity_and_describe_every_state() {
        let candidate = candidate(0, "alpha", "aaaaaaa", "First candidate");
        let expected = [
            (
                InboxRowState::Refreshing,
                "alpha #1  aaaaaaa  First candidate  PR #10 — refreshing",
            ),
            (
                InboxRowState::Bucket(ActionBucket::RefreshFailed),
                "alpha #1  aaaaaaa  First candidate  PR #10 — Refresh failed",
            ),
            (
                InboxRowState::Bucket(ActionBucket::ReadyToLand),
                "alpha #1  aaaaaaa  First candidate  PR #10 — Ready to land",
            ),
            (
                InboxRowState::Bucket(ActionBucket::ChangesRequested),
                "alpha #1  aaaaaaa  First candidate  PR #10 — Changes requested",
            ),
            (
                InboxRowState::Bucket(ActionBucket::BlockedOnCi),
                "alpha #1  aaaaaaa  First candidate  PR #10 — Blocked on CI",
            ),
            (
                InboxRowState::Bucket(ActionBucket::AwaitingReview),
                "alpha #1  aaaaaaa  First candidate  PR #10 — Awaiting review",
            ),
            (
                InboxRowState::Bucket(ActionBucket::BehindBase),
                "alpha #1  aaaaaaa  First candidate  PR #10 — Behind base",
            ),
            (
                InboxRowState::Bucket(ActionBucket::Draft),
                "alpha #1  aaaaaaa  First candidate  PR #10 — Draft",
            ),
            (
                InboxRowState::Bucket(ActionBucket::Merged),
                "alpha #1  aaaaaaa  First candidate  PR #10 — Merged",
            ),
            (
                InboxRowState::MergedHidden,
                "alpha #1  aaaaaaa  First candidate  PR #10 — merged (hidden; use --all)",
            ),
            (
                InboxRowState::ClosedHidden,
                "alpha #1  aaaaaaa  First candidate  PR #10 — closed (hidden)",
            ),
            (
                InboxRowState::RefreshFailed("timed out"),
                "alpha #1  aaaaaaa  First candidate  PR #10 — refresh failed: timed out",
            ),
        ];

        for (state, expected_message) in expected {
            assert_eq!(
                format_row_message(&candidate, "PR", state),
                expected_message
            );
        }
    }

    #[test]
    fn refresh_failure_row_neutralizes_terminal_control_characters() {
        let candidate = candidate(0, "alpha", "aaaaaaa", "First candidate");
        let message = format_row_message(
            &candidate,
            "PR",
            InboxRowState::RefreshFailed("failure: \u{1b}[31mred\u{7}"),
        );

        assert!(!message.chars().any(char::is_control));
        assert!(message.contains("refresh failed: failure: [31mred"));
    }

    #[test]
    fn completing_one_row_preserves_stable_rows_and_advances_counter() {
        let candidates = [
            candidate(0, "alpha", "aaaaaaa", "First candidate"),
            candidate(1, "beta", "bbbbbbb", "Second candidate"),
            candidate(2, "gamma", "ccccccc", "Third candidate"),
        ];
        let operations = Arc::new(Mutex::new(Vec::new()));
        let draw_target = ProgressDrawTarget::term_like(Box::new(RecordingTerm {
            operations: operations.clone(),
        }));
        let mut renderer = LiveInboxRenderer::with_draw_target(&candidates, "PR", draw_target);

        let initial_operations = operations.lock().unwrap().clone();
        let initial_frame = rendered_frames(&initial_operations)
            .into_iter()
            .find(|frame| {
                frame.contains("alpha #1  aaaaaaa  First candidate  PR #10 — refreshing")
                    && frame.contains("beta #2  bbbbbbb  Second candidate  PR #11 — refreshing")
                    && frame.contains("gamma #3  ccccccc  Third candidate  PR #12 — refreshing")
                    && frame.contains("refreshed 0/3")
            })
            .expect("the initial display should render every stable row and the counter");

        let alpha_initial = initial_frame.find("alpha #1").unwrap();
        let beta_initial = initial_frame.find("beta #2").unwrap();
        let gamma_initial = initial_frame.find("gamma #3").unwrap();
        assert!(alpha_initial < beta_initial && beta_initial < gamma_initial);

        let update_start = initial_operations.len();
        renderer.update(1, InboxRowState::Bucket(ActionBucket::ReadyToLand));

        let updated_operations = operations.lock().unwrap().clone();
        let updated_frame = rendered_frames(&updated_operations[update_start..])
            .into_iter()
            .find(|frame| {
                frame.contains("alpha #1  aaaaaaa  First candidate  PR #10 — refreshing")
                    && frame.contains("beta #2  bbbbbbb  Second candidate  PR #11 — Ready to land")
                    && frame.contains("gamma #3  ccccccc  Third candidate  PR #12 — refreshing")
                    && frame.contains("refreshed 1/3")
            })
            .expect("updating one completion should redraw all stable rows and the counter");

        let alpha_updated = updated_frame.find("alpha #1").unwrap();
        let beta_updated = updated_frame.find("beta #2").unwrap();
        let gamma_updated = updated_frame.find("gamma #3").unwrap();
        assert!(alpha_updated < beta_updated && beta_updated < gamma_updated);

        renderer.finish_and_clear();
        drop(renderer);

        let final_operations = operations.lock().unwrap().clone();
        assert!(
            final_operations.iter().any(|operation| operation == CLEAR),
            "finishing the renderer should clear its temporary terminal rows"
        );
        assert!(
            rendered_frames(&final_operations)
                .last()
                .is_some_and(|frame| frame.trim().is_empty()),
            "dropping a cleared renderer must not redraw stale terminal rows"
        );
    }

    #[test]
    fn dropping_an_active_renderer_clears_temporary_rows() {
        let candidates = [candidate(0, "alpha", "aaaaaaa", "First candidate")];
        let operations = Arc::new(Mutex::new(Vec::new()));
        let draw_target = ProgressDrawTarget::term_like(Box::new(RecordingTerm {
            operations: operations.clone(),
        }));
        let renderer = LiveInboxRenderer::with_draw_target(&candidates, "PR", draw_target);

        drop(renderer);

        let final_operations = operations.lock().unwrap().clone();
        assert!(final_operations.iter().any(|operation| operation == CLEAR));
        assert!(
            !rendered_frames(&final_operations)
                .iter()
                .any(|frame| { frame.contains("refreshed 0/1") && !frame.contains("alpha #1") }),
            "cleanup must not transiently redraw the aggregate without its stable row"
        );
        assert!(
            rendered_frames(&final_operations)
                .last()
                .is_some_and(|frame| frame.trim().is_empty()),
            "Drop must leave no live row behind after a fatal command exit"
        );
    }
}
