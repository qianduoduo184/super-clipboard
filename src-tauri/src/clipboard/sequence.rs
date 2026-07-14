use std::collections::VecDeque;

const RECENT_INTERNAL_SEQUENCE_LIMIT: usize = 32;

#[derive(Debug, PartialEq, Eq)]
pub enum SequenceDecision {
    SuppressInternal,
    IgnoreDuplicate,
    Capture,
}

#[derive(Debug, Default)]
pub struct ClipboardSequenceState {
    recent_internal_sequences: VecDeque<u32>,
    last_enqueued_sequence: Option<u32>,
}

impl ClipboardSequenceState {
    pub fn register_internal(&mut self, sequence: u32) {
        if sequence == 0 || self.recent_internal_sequences.contains(&sequence) {
            return;
        }

        if self.recent_internal_sequences.len() == RECENT_INTERNAL_SEQUENCE_LIMIT {
            self.recent_internal_sequences.pop_front();
        }
        self.recent_internal_sequences.push_back(sequence);
    }

    pub fn classify_notification(&mut self, sequence: u32) -> SequenceDecision {
        if sequence == 0 {
            return SequenceDecision::Capture;
        }
        if self.recent_internal_sequences.contains(&sequence) {
            return SequenceDecision::SuppressInternal;
        }
        if self.last_enqueued_sequence == Some(sequence) {
            return SequenceDecision::IgnoreDuplicate;
        }

        self.last_enqueued_sequence = Some(sequence);
        SequenceDecision::Capture
    }
}

pub fn is_stale_worker_event(event_sequence: u32, current_sequence: u32) -> bool {
    event_sequence != 0 && current_sequence != 0 && event_sequence != current_sequence
}

#[cfg(test)]
mod tests {
    use super::{is_stale_worker_event, ClipboardSequenceState, SequenceDecision};

    #[test]
    fn exact_internal_sequence_is_suppressed() {
        let mut state = ClipboardSequenceState::default();
        state.register_internal(41);

        assert_eq!(
            state.classify_notification(41),
            SequenceDecision::SuppressInternal
        );
    }

    #[test]
    fn duplicate_external_notification_is_ignored() {
        let mut state = ClipboardSequenceState::default();

        assert_eq!(state.classify_notification(42), SequenceDecision::Capture);
        assert_eq!(
            state.classify_notification(42),
            SequenceDecision::IgnoreDuplicate
        );
    }

    #[test]
    fn two_queued_internal_writes_are_both_suppressed() {
        let mut state = ClipboardSequenceState::default();
        state.register_internal(43);
        state.register_internal(44);

        assert_eq!(
            state.classify_notification(43),
            SequenceDecision::SuppressInternal
        );
        assert_eq!(
            state.classify_notification(44),
            SequenceDecision::SuppressInternal
        );
    }

    #[test]
    fn external_sequence_after_internal_write_is_captured() {
        let mut state = ClipboardSequenceState::default();
        state.register_internal(45);

        assert_eq!(
            state.classify_notification(45),
            SequenceDecision::SuppressInternal
        );
        assert_eq!(state.classify_notification(46), SequenceDecision::Capture);
    }

    #[test]
    fn stale_worker_event_is_rejected() {
        assert!(is_stale_worker_event(47, 48));
        assert!(!is_stale_worker_event(47, 47));
        assert!(!is_stale_worker_event(47, 0));
        assert!(!is_stale_worker_event(0, 48));
    }

    #[test]
    fn recent_internal_sequences_are_bounded() {
        let mut state = ClipboardSequenceState::default();
        for sequence in 1..=33 {
            state.register_internal(sequence);
        }

        assert_eq!(state.classify_notification(1), SequenceDecision::Capture);
        assert_eq!(
            state.classify_notification(2),
            SequenceDecision::SuppressInternal
        );
    }

    #[test]
    fn zero_sequence_is_never_registered_and_always_fails_open() {
        let mut state = ClipboardSequenceState::default();
        state.register_internal(0);

        assert_eq!(state.classify_notification(0), SequenceDecision::Capture);
        assert_eq!(state.classify_notification(0), SequenceDecision::Capture);
    }
}
