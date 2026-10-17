//! A fixed-window counter: at most `limit` units per hour.
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(3600);

pub struct HourlyBudget {
    limit: u32,
    state: Mutex<(Instant, u32)>,
}

impl HourlyBudget {
    pub fn new(limit: u32) -> Self {
        Self {
            limit,
            state: Mutex::new((Instant::now(), 0)),
        }
    }

    /// Takes one unit. Returns the seconds until the window resets if the
    /// budget is exhausted.
    pub fn try_take(&self) -> Result<(), u64> {
        self.update(true)
    }

    /// Checks for a free unit without taking it.
    pub fn check(&self) -> Result<(), u64> {
        self.update(false)
    }

    fn update(&self, take: bool) -> Result<(), u64> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.0.elapsed() >= WINDOW {
            *state = (Instant::now(), 0);
        }
        if state.1 >= self.limit {
            return Err(WINDOW.saturating_sub(state.0.elapsed()).as_secs().max(1));
        }
        if take {
            state.1 += 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_the_limit() {
        let budget = HourlyBudget::new(2);
        assert!(budget.check().is_ok());
        assert!(budget.check().is_ok(), "checking does not consume");
        assert!(budget.try_take().is_ok());
        assert!(budget.try_take().is_ok());
        let retry = budget.try_take().unwrap_err();
        assert!((1..=3600).contains(&retry));
    }
}
