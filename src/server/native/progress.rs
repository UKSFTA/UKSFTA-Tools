//! Progress for the segmented transfer. Every bar draws on stderr, so a
//! piped stdout cannot break it, and `hidden` covers the no-flag case.

use std::sync::Arc;

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget};

/// One bar per worker plus a total bar.
pub struct SegmentedProgress {
    total: ProgressBar,
    workers: Vec<ProgressBar>,
    _multi: MultiProgress,
}

impl SegmentedProgress {
    pub fn new(size: u64, connections: usize, show: bool) -> Arc<Self> {
        let target = if show {
            ProgressDrawTarget::stderr()
        } else {
            ProgressDrawTarget::hidden()
        };
        let multi = MultiProgress::with_draw_target(target);
        let total = multi.add(ProgressBar::new(size));
        let workers = (0..connections.max(1))
            .map(|_| multi.add(ProgressBar::new(0)))
            .collect();
        Arc::new(Self {
            total,
            workers,
            _multi: multi,
        })
    }

    pub fn advance(&self, worker: usize, bytes: u64) {
        if let Some(bar) = self.workers.get(worker) {
            bar.inc(bytes);
        }
        self.total.inc(bytes);
    }

    pub fn finish(&self) {
        for bar in &self.workers {
            bar.finish_and_clear();
        }
        self.total.finish_and_clear();
    }
}
