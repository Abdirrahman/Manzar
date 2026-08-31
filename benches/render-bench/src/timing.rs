//! A small measurement loop: warm up, then take enough samples to report a
//! median rather than a single lucky run.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Sample {
    pub samples: usize,
    pub min_ms: f64,
    pub median_ms: f64,
    pub mean_ms: f64,
    pub max_ms: f64,
}

impl Sample {
    /// Speed-up of `self` over `baseline`, as a multiple. Medians are compared
    /// so one descheduled run cannot manufacture a result.
    pub fn speedup_over(&self, baseline: &Sample) -> f64 {
        baseline.median_ms / self.median_ms
    }
}

/// Runs `body` until both the warmup budget and the sample count are met.
///
/// Large-image work is slow enough that a fixed sample count would make the
/// suite unusable, so the budget caps total wall time and the loop settles for
/// fewer samples on the expensive fixtures.
pub fn measure<T>(
    warmup: Duration,
    min_samples: usize,
    budget: Duration,
    mut body: impl FnMut() -> T,
) -> Sample {
    let warmup_start = Instant::now();
    loop {
        std::hint::black_box(body());
        if warmup_start.elapsed() >= warmup {
            break;
        }
    }

    let mut timings = Vec::with_capacity(min_samples);
    let started = Instant::now();

    // Always take at least one sample, then keep going until either the sample
    // count is met or the time budget runs out. Without the budget the
    // 61-megapixel fixtures alone would dominate a run.
    while timings.len() < min_samples {
        let iteration = Instant::now();
        std::hint::black_box(body());
        timings.push(iteration.elapsed().as_secs_f64() * 1000.0);

        if started.elapsed() >= budget {
            break;
        }
    }

    summarise(&mut timings)
}

fn summarise(timings: &mut [f64]) -> Sample {
    timings.sort_by(|a, b| a.partial_cmp(b).expect("no NaN timings"));
    let median = if timings.len() % 2 == 0 {
        (timings[timings.len() / 2 - 1] + timings[timings.len() / 2]) / 2.0
    } else {
        timings[timings.len() / 2]
    };

    Sample {
        samples: timings.len(),
        min_ms: timings[0],
        median_ms: median,
        mean_ms: timings.iter().sum::<f64>() / timings.len() as f64,
        max_ms: timings[timings.len() - 1],
    }
}
