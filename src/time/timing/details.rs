//! Control whether frame checkpoint paths retain their detailed hierarchy.
use super::StepTimes;

impl StepTimes {
    pub fn enable_details(&mut self, enabled: bool) { self.detailed = enabled; }
}
