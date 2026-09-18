//! Per-pass UI work budget. Native pools live in terminator_core::async_service.
use std::time::{Duration, Instant};
/// A pass can exceed the time limit by one handler; handlers must remain cheap.
pub struct ResultBudget {
    started: Instant,
    processed: usize,
}
impl ResultBudget {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            processed: 0,
        }
    }
    pub fn next(&mut self) -> bool {
        if self.processed >= 64 || self.started.elapsed() >= Duration::from_millis(2) {
            return false;
        }
        self.processed += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completion_pass_has_count_and_time_limits() {
        let mut budget = ResultBudget::new();
        budget.processed = 64;
        assert!(!budget.next());
        budget.processed = 0;
        budget.started = Instant::now() - Duration::from_millis(3);
        assert!(!budget.next());
    }
}
