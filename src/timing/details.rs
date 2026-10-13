//! Optional frame sub-steps. Repeated passes accumulate locally and publish one child row per pass per frame,
//! keeping traces bounded even when millions of texels are projected. Disabled passes read no clock.
use super::StepTimes;
use std::time::{Duration, Instant};

impl StepTimes {
    pub fn enable_details(&mut self, enabled: bool) { self.detailed = enabled; }

    /// Time a single sub-step only when the panel, trace or benchmark requests detail.
    pub fn measure_detail<T>(&mut self, name: &'static str, run: impl FnOnce() -> T) -> T {
        if self.detailed || self.trace.is_some() { self.measure(name, run) } else { run() }
    }

    /// Create a fixed-size accumulator for interleaved passes. Publish after the loop, inside its parent scope.
    pub fn detail_batch<const N: usize>(&self, names: [&'static str; N]) -> DetailBatch<N> {
        DetailBatch { names, elapsed: [None; N], enabled: self.detailed || self.trace.is_some() }
    }
}

pub struct DetailBatch<const N: usize> {
    names: [&'static str; N],
    elapsed: [Option<Duration>; N],
    enabled: bool,
}

impl<const N: usize> DetailBatch<N> {
    #[inline]
    pub fn measure<T>(&mut self, pass: usize, run: impl FnOnce() -> T) -> T {
        if !self.enabled { return run(); }
        let start = Instant::now();
        let result = run();
        *self.elapsed[pass].get_or_insert(Duration::ZERO) += start.elapsed();
        result
    }

    /// Trace rows represent totals over the loop, in declared pass order. Unexecuted passes remain absent.
    pub fn publish(self, times: &mut StepTimes) {
        for (name, elapsed) in self.names.into_iter().zip(self.elapsed) {
            let Some(elapsed) = elapsed else { continue; };
            let seconds = elapsed.as_secs_f64();
            let index = times.start_trace(name);
            times.finish_trace(index, seconds);
            times.record(name, seconds);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_passes_publish_one_child_with_the_sum_and_skip_unexecuted_passes() {
        let mut times = StepTimes::with_trace(true);
        times.begin_frame();
        times.measure_steps("Projection", |times| {
            let mut batch = times.detail_batch(["Geometry", "Raster", "Unused"]);
            for _ in 0..1000 { assert_eq!(batch.measure(0, || 42), 42); }
            batch.elapsed[0] = Some(Duration::from_millis(2));
            batch.elapsed[1] = Some(Duration::from_millis(3));
            batch.publish(times);
        });
        assert_eq!(times.steps().iter().map(|step| (step.name, step.depth)).collect::<Vec<_>>(), [("Projection", 0), ("Geometry", 1), ("Raster", 1)]);
        assert_eq!(times.steps()[1].average_seconds, 0.002);
        assert_eq!(times.steps()[2].average_seconds, 0.003);
        assert_eq!(times.trace().unwrap().steps.len(), 3);
        times.begin_frame();
        times.measure_steps("Projection", |times| times.detail_batch(["Geometry"]).publish(times));
        times.begin_frame();
        assert!(times.steps()[1].average_seconds < 0.002);                                // a skipped frame is not recorded as another positive sample
    }

    #[test]
    fn disabled_detail_runs_the_work_without_creating_rows_or_collecting_samples() {
        let mut times = StepTimes::default();
        assert_eq!(times.measure_detail("One", || 7), 7);
        let mut batch = times.detail_batch(["Many"]);
        assert_eq!(batch.measure(0, || 9), 9);
        assert!(batch.elapsed[0].is_none());
        batch.publish(&mut times);
        assert!(times.steps().is_empty());
        times.enable_details(true);
        times.measure_detail("Enabled", || ());
        assert_eq!(times.steps()[0].name, "Enabled");
    }
}
