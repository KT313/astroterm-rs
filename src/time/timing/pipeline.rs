//! Start frame measurements and expose the completed timing averages.
use super::{StepTime, StepTimes};

impl StepTimes {
    /// Start a frame. Repeated paths contribute to one frame total before smoothing.
    /// Startup segments accumulate totals without frame-to-frame smoothing.
    pub fn begin_frame(&mut self) {
        assert!(!self.has_active_checkpoint(), "finish checkpoint timing with END before starting a frame");
        self.clear_checkpoint_trace();
        self.reset_frame_samples(); // carry completed averages forward and clear this frame's totals
        self.per_frame = true;
    }

    /// The steps measured so far, with their averages.
    pub fn steps(&self) -> &[StepTime] {
        &self.steps
    }
}
