use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

use super::InboxCandidate;
use crate::provider::InboxSnapshot;

pub(super) const MAX_INBOX_REFRESH_WORKERS: usize = 4;

#[derive(Debug)]
pub(super) struct InboxCompletion {
    pub candidate: InboxCandidate,
    pub result: std::result::Result<InboxSnapshot, String>,
}

pub(super) fn refresh_candidates<F, C>(
    candidates: &[InboxCandidate],
    refresh: F,
    mut on_completion: C,
) where
    F: Fn(&InboxCandidate) -> std::result::Result<InboxSnapshot, String> + Sync,
    C: FnMut(InboxCompletion),
{
    let worker_count = candidates.len().min(MAX_INBOX_REFRESH_WORKERS);
    if worker_count == 0 {
        return;
    }

    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel();

    thread::scope(|scope| {
        for _ in 0..worker_count {
            let sender = sender.clone();
            let next = &next;
            let refresh = &refresh;
            scope.spawn(move || loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(candidate) = candidates.get(index).cloned() else {
                    break;
                };
                let result = refresh(&candidate);
                if sender.send(InboxCompletion { candidate, result }).is_err() {
                    break;
                }
            });
        }

        drop(sender);
        for completion in receiver {
            on_completion(completion);
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Barrier, Condvar, Mutex};
    use std::thread;

    use super::refresh_candidates;
    use crate::commands::inbox::InboxCandidate;
    use crate::provider::{InboxSnapshot, PrState};

    fn candidate(discovery_index: usize) -> InboxCandidate {
        InboxCandidate {
            discovery_index,
            stack_name: "stack".to_string(),
            position: discovery_index + 1,
            short_sha: format!("{discovery_index:07x}"),
            title: format!("Candidate {discovery_index}"),
            pr_number: discovery_index as u64 + 1,
            behind_base: None,
        }
    }

    fn candidates(count: usize) -> Vec<InboxCandidate> {
        (0..count).map(candidate).collect()
    }

    fn snapshot() -> InboxSnapshot {
        InboxSnapshot {
            state: PrState::Open,
            url: "https://example.com/pull/1".to_string(),
            approved: false,
            changes_requested: false,
            mergeable: true,
            ci_status: None,
        }
    }

    #[test]
    fn refreshes_at_most_four_candidates_at_once() {
        let active = AtomicUsize::new(0);
        let maximum = AtomicUsize::new(0);
        let first_wave = Barrier::new(4);
        let candidates = candidates(8);

        refresh_candidates(
            &candidates,
            |candidate| {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(now, Ordering::SeqCst);
                if candidate.discovery_index < 4 {
                    first_wave.wait();
                }
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(snapshot())
            },
            |_| {},
        );

        assert_eq!(maximum.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn does_nothing_when_there_are_no_candidates() {
        let refresh_count = AtomicUsize::new(0);
        let mut completion_count = 0;

        refresh_candidates(
            &[],
            |_| {
                refresh_count.fetch_add(1, Ordering::SeqCst);
                Ok(snapshot())
            },
            |_| completion_count += 1,
        );

        assert_eq!(refresh_count.load(Ordering::SeqCst), 0);
        assert_eq!(completion_count, 0);
    }

    #[test]
    fn refreshes_fewer_than_four_candidates_concurrently() {
        let active = AtomicUsize::new(0);
        let maximum = AtomicUsize::new(0);
        let first_wave = Barrier::new(3);

        refresh_candidates(
            &candidates(3),
            |_| {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(now, Ordering::SeqCst);
                first_wave.wait();
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(snapshot())
            },
            |_| {},
        );

        assert_eq!(maximum.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn refreshes_every_candidate_exactly_once() {
        let refresh_counts: Vec<AtomicUsize> = (0..8).map(|_| AtomicUsize::new(0)).collect();
        let mut completed = Vec::new();

        refresh_candidates(
            &candidates(8),
            |candidate| {
                refresh_counts[candidate.discovery_index].fetch_add(1, Ordering::SeqCst);
                Ok(snapshot())
            },
            |completion| completed.push(completion.candidate.discovery_index),
        );

        completed.sort_unstable();
        assert_eq!(completed, (0..8).collect::<Vec<_>>());
        assert!(refresh_counts
            .iter()
            .all(|count| count.load(Ordering::SeqCst) == 1));
    }

    #[test]
    fn worker_errors_do_not_cancel_later_candidates() {
        let mut completions = Vec::new();

        refresh_candidates(
            &candidates(8),
            |candidate| {
                if candidate.discovery_index == 0 {
                    Err("refresh failed".to_string())
                } else {
                    Ok(snapshot())
                }
            },
            |completion| {
                completions.push((
                    completion.candidate.discovery_index,
                    completion.result.map(|_| ()),
                ));
            },
        );

        completions.sort_by_key(|(index, _)| *index);
        assert_eq!(completions.len(), 8);
        assert_eq!(completions[0], (0, Err("refresh failed".to_string())));
        assert!(completions[1..].iter().all(|(_, result)| result.is_ok()));
    }

    #[test]
    fn reports_fast_later_candidate_before_blocked_first_candidate() {
        let gate = (Mutex::new(0_u8), Condvar::new());
        let mut order = Vec::new();

        refresh_candidates(
            &candidates(2),
            |candidate| {
                if candidate.discovery_index == 0 {
                    let (lock, ready) = &gate;
                    let released = lock.lock().unwrap();
                    drop(ready.wait_while(released, |state| *state < 2).unwrap());
                } else {
                    *gate.0.lock().unwrap() = 1;
                }
                Ok(snapshot())
            },
            |completion| {
                let index = completion.candidate.discovery_index;
                order.push(index);
                if index == 1 {
                    *gate.0.lock().unwrap() = 2;
                    gate.1.notify_one();
                }
            },
        );

        assert_eq!(order, vec![1, 0]);
    }

    #[test]
    fn runs_completion_callbacks_on_the_coordinator_thread() {
        let coordinator = thread::current().id();
        let mut callback_threads = Vec::new();

        refresh_candidates(
            &candidates(8),
            |_| Ok(snapshot()),
            |_| callback_threads.push(thread::current().id()),
        );

        assert_eq!(callback_threads, vec![coordinator; 8]);
    }
}
