//! Progress for the segmented transfer. Bars draw on stderr, so a piped
//! or JSON stdout stays clean.

use std::sync::Arc;
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};

use super::segment::Chunk;

/// One total bar plus one bar per connection, in the download-manager style.
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
        total.set_style(
            ProgressStyle::with_template(
                "{spinner} total {bar:32} {percent:>3}% {bytes:>10}/{total_bytes:<10} \
                 {bytes_per_sec:>10} elapsed {elapsed_precise} ETA {eta_precise}",
            )
            .expect("the total progress template is valid")
            .progress_chars("=>-"),
        );
        let workers: Vec<ProgressBar> = (0..connections.max(1))
            .map(|index| {
                let bar = multi.add(ProgressBar::new(0));
                bar.set_style(
                    ProgressStyle::with_template(
                        "{spinner} conn {prefix:>2} {bar:24} {bytes:>9}/{total_bytes:<9} \
                         {bytes_per_sec:>9} ETA {eta_precise} {msg}",
                    )
                    .expect("the worker progress template is valid")
                    .progress_chars("=>-"),
                );
                bar.set_prefix((index + 1).to_string());
                bar
            })
            .collect();
        if show {
            total.enable_steady_tick(Duration::from_millis(250));
            for bar in &workers {
                bar.enable_steady_tick(Duration::from_millis(250));
            }
        }
        Arc::new(Self {
            total,
            workers,
            _multi: multi,
        })
    }

    /// Start a new range on one connection.
    pub fn start_range(&self, worker: usize, chunk: &Chunk) {
        if let Some(bar) = self.workers.get(worker) {
            bar.set_length(u64::from(chunk.len));
            bar.set_position(0);
            bar.set_message(format!("range {}", chunk.index + 1));
        }
    }

    /// Count bytes after they are acknowledged and, when supported, synced.
    pub fn advance(&self, worker: usize, bytes: u64) {
        if let Some(bar) = self.workers.get(worker) {
            bar.inc(bytes);
        }
        self.total.inc(bytes);
    }

    /// A range finished; leave its bar at 100% until the next range.
    pub fn finish_range(&self, worker: usize) {
        if let Some(bar) = self.workers.get(worker) {
            bar.set_position(bar.length().unwrap_or(0));
            bar.set_message("done");
        }
    }

    /// A range failed and goes back to the queue.
    pub fn reset_range(&self, worker: usize) {
        if let Some(bar) = self.workers.get(worker) {
            bar.set_position(0);
            bar.set_message("retry");
        }
    }

    pub fn finish(&self) {
        for bar in &self.workers {
            bar.finish_and_clear();
        }
        self.total.finish_and_clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hidden_bar_advances_without_panicking() {
        let progress = SegmentedProgress::new(100, 4, false);
        let chunk = Chunk {
            index: 0,
            start: 0,
            len: 50,
        };
        progress.start_range(0, &chunk);
        progress.advance(0, 50);
        progress.finish_range(0);
        progress.reset_range(1);
        progress.finish();
    }
}
