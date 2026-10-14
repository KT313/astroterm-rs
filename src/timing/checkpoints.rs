//! Sequential timing boundaries. Dotted paths form a tree; each interval contributes to its ancestors once.
use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::StepTimes;

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Checkpoints {
    paths: HashMap<&'static str, usize>,
    nodes: Vec<Node>,
    active: Option<(usize, Instant)>,
}

#[derive(Clone, Debug, PartialEq)]
struct Node {
    name: &'static str,
    parent: Option<usize>,
    elapsed: Option<Duration>,
    trace_index: Option<usize>,
}

impl StepTimes {
    /// Finish the preceding interval and begin `path`. `END` finishes without starting another interval.
    /// Repeated paths accumulate within the frame; dotted ancestors include all descendant intervals.
    /// Without detailed diagnostics, paths collapse to their top-level stage and child calls read no clock.
    pub fn measure_checkpoint(&mut self, path: &'static str) {
        self.measure_at(path, Instant::now);
    }

    fn measure_at(&mut self, path: &'static str, now: impl FnOnce() -> Instant) {
        // Keep ordinary rendering cheap: only transitions between top-level stages need a clock read.
        let path = if self.detailed || self.trace.is_some() {
            path
        } else {
            path.split_once('.').map_or(path, |(root, _)| root)
        };
        if path != "END" && let Some((active, _)) = &self.checkpoints.active
            && self.checkpoints.paths.get(path) == Some(active)
        {
            return;
        }

        // Use one boundary instant so adjacent measured intervals have neither gaps nor overlap.
        let now = now();
        self.finish_checkpoint(now);
        if path == "END" {
            self.publish_checkpoints();
            return;
        }

        // Register each ancestor before its children; reuse the tree across frames.
        assert!(!path.split('.').any(str::is_empty), "timing paths must contain nonempty components");
        let index = self.register_checkpoint(path);
        self.checkpoints.active = Some((index, now));
    }

    fn register_checkpoint(&mut self, path: &'static str) -> usize {
        let index = if let Some(&index) = self.checkpoints.paths.get(path) {
            index
        } else {
            let (parent, name) = match path.rsplit_once('.') {
                Some((parent, name)) => (Some(self.register_checkpoint(parent)), name),
                None => (None, path),
            };
            let index = self.checkpoints.nodes.len();
            self.checkpoints.nodes.push(Node { name, parent, elapsed: None, trace_index: None });
            self.checkpoints.paths.insert(path, index);
            index
        };

        // Reserve bounded trace rows when a path first runs in this capture segment.
        // Parents may already exist in the persistent tree but still need a row in a new frame's trace.
        if self.trace.is_some() && self.checkpoints.nodes[index].trace_index.is_none() {
            if let Some((parent, _)) = path.rsplit_once('.') { self.register_checkpoint(parent); }
            let node = &self.checkpoints.nodes[index];
            let parent = node.parent.and_then(|parent| self.checkpoints.nodes[parent].trace_index);
            let trace = self.trace.as_mut().unwrap();
            let trace_index = parent.and_then(|parent| {
                (parent + 1..trace.steps.len()).find(|&index| trace.steps[index].depth <= trace.steps[parent].depth)
            }).unwrap_or(trace.steps.len());
            let row = super::TraceStep {
                name: node.name,
                depth: parent.map_or(0, |parent| trace.steps[parent].depth + 1),
                seconds: 0.0,
                details: Vec::new(),
                direct_diagnostic_seconds: 0.0,
                parent,
            };

            // A newly encountered child may belong to an earlier branch. Keep exports in tree order,
            // updating stored indices when insertion moves rows already used by another checkpoint.
            for row in &mut trace.steps {
                if let Some(parent) = &mut row.parent && *parent >= trace_index { *parent += 1; }
            }
            for active in &mut trace.active {
                if *active >= trace_index { *active += 1; }
            }
            trace.steps.insert(trace_index, row);
            for node in &mut self.checkpoints.nodes {
                if let Some(index) = &mut node.trace_index && *index >= trace_index { *index += 1; }
            }
            self.checkpoints.nodes[index].trace_index = Some(trace_index);
        }
        index
    }

    fn finish_checkpoint(&mut self, now: Instant) {
        // Attribute the completed interval to its leaf and every ancestor, without counting any interval twice.
        let Some((index, started)) = self.checkpoints.active.take() else { return; };
        let elapsed = now.duration_since(started);
        let mut next = Some(index);
        while let Some(index) = next {
            let node = &mut self.checkpoints.nodes[index];
            *node.elapsed.get_or_insert(Duration::ZERO) += elapsed;
            next = node.parent;
        }
    }

    fn publish_checkpoints(&mut self) {
        // Feed each frame total into the existing smoothing once; END itself never appears as a timing row.
        let saved_parents = std::mem::take(&mut self.parents);
        for index in 0..self.checkpoints.nodes.len() {
            let Some(elapsed) = self.checkpoints.nodes[index].elapsed.take() else { continue; };
            self.parents.clear();
            let mut parent = self.checkpoints.nodes[index].parent;
            while let Some(index) = parent {
                self.parents.push(self.checkpoints.nodes[index].name);
                parent = self.checkpoints.nodes[index].parent;
            }
            self.parents.reverse();
            let node = &self.checkpoints.nodes[index];
            let name = node.name;
            let trace_index = node.trace_index;
            let seconds = elapsed.as_secs_f64();
            if let Some(trace_index) = trace_index {
                self.trace.as_mut().unwrap().steps[trace_index].seconds += seconds;
            }
            let previous_rows = self.steps.len();
            self.record(name, seconds);
            self.group_new_checkpoint_row(previous_rows);
        }
        self.parents = saved_parents;
    }

    fn group_new_checkpoint_row(&mut self, previous_rows: usize) {
        // A child first encountered on a later frame still belongs below its parent in the timing panel.
        if self.steps.len() == previous_rows || self.parents.is_empty() { return; }
        let parent_name = self.parents.last().unwrap();
        let ancestors = &self.parents[..self.parents.len() - 1];
        let parent = self.steps.iter().zip(&self.records).position(|(step, record)| {
            step.name == *parent_name && record.parents == ancestors
        }).expect("checkpoint ancestors are published before their children");
        let parent_depth = self.steps[parent].depth;
        let insertion = (parent + 1..previous_rows).find(|&index| self.steps[index].depth <= parent_depth);
        if let Some(insertion) = insertion {
            self.steps[insertion..].rotate_right(1);
            self.records[insertion..].rotate_right(1);
        }
    }

    pub(super) fn checkpoint_trace_index(&self, path: &str) -> Option<usize> {
        let index = self.checkpoints.paths.get(path)?;
        self.checkpoints.nodes[*index].trace_index
    }

    pub(super) fn has_active_checkpoint(&self) -> bool { self.checkpoints.active.is_some() }

    pub(super) fn active_checkpoint_trace_index(&self) -> Option<usize> {
        let (index, _) = self.checkpoints.active.as_ref()?;
        self.checkpoints.nodes[*index].trace_index
    }

    pub(super) fn clear_checkpoint_trace(&mut self) {
        for node in &mut self.checkpoints.nodes { node.trace_index = None; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::TIMING_EMA_FACTOR;

    fn advance(times: &mut StepTimes, origin: &Instant, milliseconds: u64, path: &'static str) {
        times.measure_at(path, || *origin + Duration::from_millis(milliseconds));
    }

    #[test]
    fn nested_paths_sum_repeated_intervals_without_double_counting_or_idle_time() {
        let origin = Instant::now();
        let mut times = StepTimes::with_trace(true);
        times.begin_frame();
        advance(&mut times, &origin, 0, "Solar system.Earth.State");
        advance(&mut times, &origin, 2, "Solar system.Sun");
        advance(&mut times, &origin, 5, "Solar system.Earth.State");
        advance(&mut times, &origin, 9, "Observation.Earth.State");
        assert!(times.capture_steps().is_err());
        advance(&mut times, &origin, 10, "END");
        advance(&mut times, &origin, 100, "END"); // a second END cannot charge idle time

        let rows = &times.trace().unwrap().steps;
        assert_eq!(rows.iter().map(|row| (row.name, row.depth, row.seconds)).collect::<Vec<_>>(), [
            ("Solar system", 0, 0.009), ("Earth", 1, 0.006), ("State", 2, 0.006), ("Sun", 1, 0.003),
            ("Observation", 0, 0.001), ("Earth", 1, 0.001), ("State", 2, 0.001),
        ]);
        assert_eq!(rows[2].parent, Some(1));
        assert_eq!(rows[6].parent, Some(5));
        assert_eq!(times.steps().len(), 7);
        assert!(times.capture_steps().is_ok());
    }

    #[test]
    fn thousands_of_repetitions_use_one_row_and_smooth_one_total_per_frame() {
        let origin = Instant::now();
        let mut times = StepTimes::with_trace(true);
        times.begin_frame();
        for index in 0..1000 {
            advance(&mut times, &origin, index * 3, "Regions.Draw");
            advance(&mut times, &origin, index * 3 + 1, "Regions.Merge");
        }
        advance(&mut times, &origin, 3000, "END");
        assert_eq!(times.trace().unwrap().steps.len(), 3);
        assert_eq!(times.steps()[0].average_seconds, 3.0);
        assert_eq!(times.steps()[1].average_seconds, 1.0);
        assert_eq!(times.steps()[2].average_seconds, 2.0);

        times.clear_capture_segment().unwrap();
        times.begin_frame();
        advance(&mut times, &origin, 10000, "Regions.Draw");
        advance(&mut times, &origin, 12000, "END");
        assert_eq!(times.steps()[0].average_seconds, 3.0 * TIMING_EMA_FACTOR + 2.0 * (1.0 - TIMING_EMA_FACTOR));
        assert_eq!(times.trace().unwrap().steps.len(), 2);
        assert_eq!(times.trace().unwrap().steps[0].seconds, 2.0);
        assert_eq!(times.trace().unwrap().steps[1].parent, Some(0));
        times.begin_frame();
        assert_eq!(times.steps()[2].average_seconds, 2.0 * TIMING_EMA_FACTOR);
    }

    #[test]
    fn ordinary_frames_skip_child_clock_reads_but_keep_stage_totals() {
        let origin = Instant::now();
        let mut times = StepTimes::default();
        times.begin_frame();
        advance(&mut times, &origin, 0, "Solar system.Earth");
        times.measure_at("Solar system.Sun", || panic!("child checkpoints need no clock without detail"));
        advance(&mut times, &origin, 3, "Observation.Earth");
        advance(&mut times, &origin, 5, "END");
        assert_eq!(times.steps().iter().map(|row| (row.name, row.depth, row.average_seconds)).collect::<Vec<_>>(), [
            ("Solar system", 0, 0.003), ("Observation", 0, 0.002),
        ]);
    }

    #[test]
    fn detailed_panel_works_without_trace_and_paths_accept_arbitrary_depth() {
        let origin = Instant::now();
        let mut times = StepTimes::default();
        times.enable_details(true);
        times.begin_frame();
        advance(&mut times, &origin, 0, "A.B.C.D.E");
        advance(&mut times, &origin, 1, "END");
        assert!(times.trace().is_none());
        assert_eq!(times.steps().len(), 5);
        assert_eq!(times.steps()[4].depth, 4);
        assert!(times.steps().iter().all(|row| row.average_seconds == 0.001));
    }

    #[test]
    fn children_discovered_in_later_frames_stay_below_their_own_parent() {
        let origin = Instant::now();
        let mut times = StepTimes::with_trace(true);
        times.begin_frame();
        advance(&mut times, &origin, 0, "First");
        advance(&mut times, &origin, 1, "Second");
        advance(&mut times, &origin, 2, "END");
        times.clear_capture_segment().unwrap();
        times.begin_frame();
        advance(&mut times, &origin, 3, "First.New.Child");
        advance(&mut times, &origin, 4, "Second.New");
        advance(&mut times, &origin, 5, "END");
        assert_eq!(times.steps().iter().map(|row| (row.name, row.depth)).collect::<Vec<_>>(), [
            ("First", 0), ("New", 1), ("Child", 2), ("Second", 0), ("New", 1),
        ]);
    }

    #[test]
    fn interleaved_branches_export_in_tree_order_with_correct_parent_indices() {
        let origin = Instant::now();
        let mut times = StepTimes::with_trace(true);
        times.begin_frame();
        advance(&mut times, &origin, 0, "A.First");
        advance(&mut times, &origin, 1, "B.First");
        advance(&mut times, &origin, 2, "A.Second.Child");
        times.describe("B.First", || "belongs to B even after insertion".into());
        advance(&mut times, &origin, 3, "B.First");
        advance(&mut times, &origin, 4, "END");

        let rows = &times.trace().unwrap().steps;
        assert_eq!(rows.iter().map(|row| (row.name, row.parent, row.seconds)).collect::<Vec<_>>(), [
            ("A", None, 0.002), ("First", Some(0), 0.001), ("Second", Some(0), 0.001),
            ("Child", Some(2), 0.001), ("B", None, 0.002), ("First", Some(4), 0.002),
        ]);
        assert_eq!(rows[5].details, ["belongs to B even after insertion"]);
        assert!(times.capture_steps().is_ok());
    }
}
