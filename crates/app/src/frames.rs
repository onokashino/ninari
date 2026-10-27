//! How long the first frame took from the start of the process, always; and with NINARI_FRAMES=1
//! how the window keeps up, written to the log every two seconds while it draws:
//! how long a frame takes to build (Root's render to the end of paint) and the gaps between
//! frames. A gap well over the usual one is a frame the screen did not get in time.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Instant;

/// When the process started, set first thing in main.
pub static STARTED: OnceLock<Instant> = OnceLock::new();

thread_local! {
    static FIRST: Cell<bool> = const { Cell::new(true) };
}

use gpui::{IntoElement, Styled, canvas};

#[derive(Default)]
struct Inner {
    start: Option<Instant>,
    last: Option<Instant>,
    built: Vec<f32>,
    gaps: Vec<f32>,
    since: Option<Instant>,
}

#[derive(Clone, Default)]
pub struct Frames(Option<Rc<RefCell<Inner>>>);

impl Frames {
    pub fn from_env() -> Self {
        Self(
            std::env::var("NINARI_FRAMES")
                .is_ok_and(|v| !v.is_empty() && v != "0")
                .then(Rc::default),
        )
    }

    /// At the start of Root's render.
    pub fn begin(&self) {
        if FIRST.with(|f| f.replace(false))
            && let Some(start) = STARTED.get()
        {
            tracing::info!("first frame {} ms after start", start.elapsed().as_millis());
        }
        let Some(inner) = &self.0 else { return };
        let mut s = inner.borrow_mut();
        let now = Instant::now();
        if let Some(last) = s.last.replace(now) {
            let gap = (now - last).as_secs_f32() * 1000.;
            // Longer than this the window was idle, not late.
            if gap < 100. {
                s.gaps.push(gap);
            }
            // A late frame, with when it was, to match against samples of the main thread.
            if (25. ..100.).contains(&gap)
                && let Some(started) = STARTED.get()
            {
                tracing::info!(
                    "frames: late {gap:.0} ms, ended at {} ms",
                    (now - *started).as_millis()
                );
            }
        }
        s.start = Some(now);
        s.since.get_or_insert(now);
    }

    /// The last thing Root paints: the frame is built.
    pub fn end(&self) -> Option<impl IntoElement + use<>> {
        let inner = self.0.clone()?;
        Some(
            canvas(
                |_, _, _| {},
                move |_, _, _, _| {
                    let mut s = inner.borrow_mut();
                    if let Some(start) = s.start.take() {
                        s.built.push(start.elapsed().as_secs_f32() * 1000.);
                    }
                    if s.since.is_some_and(|t| t.elapsed().as_secs() >= 2) {
                        s.since = None;
                        let built = std::mem::take(&mut s.built);
                        let gaps = std::mem::take(&mut s.gaps);
                        if let Some(line) = report(built, gaps) {
                            tracing::info!("frames: {line}");
                        }
                    }
                },
            )
            .absolute()
            .size_0(),
        )
    }
}

fn report(mut built: Vec<f32>, mut gaps: Vec<f32>) -> Option<String> {
    if built.is_empty() {
        return None;
    }
    if built.len() < 10 || gaps.is_empty() {
        // Few frames in two seconds: the window was meant to be still. Anything here redraws
        // while nothing moves.
        return Some(format!("{} while still", built.len()));
    }
    let pct = |v: &[f32], p: f32| v[((v.len() - 1) as f32 * p) as usize];
    built.sort_by(f32::total_cmp);
    gaps.sort_by(f32::total_cmp);
    let usual = pct(&gaps, 0.5);
    let late = gaps.iter().filter(|g| **g > usual * 1.5).count();
    Some(format!(
        "{} built in avg {:.1} ms, p95 {:.1}, max {:.1}; gaps median {usual:.1} ms, p95 {:.1}, max {:.1}; late {late}",
        built.len(),
        built.iter().sum::<f32>() / built.len() as f32,
        pct(&built, 0.95),
        built[built.len() - 1],
        pct(&gaps, 0.95),
        gaps[gaps.len() - 1],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_counts_late_frames() {
        let built = vec![2.; 20];
        let mut gaps = vec![7.; 18];
        gaps.extend([20., 30.]);
        let line = report(built, gaps).unwrap();
        assert!(line.starts_with("20 built in avg 2.0 ms"), "{line}");
        assert!(line.ends_with("late 2"), "{line}");
        assert_eq!(
            report(vec![1.; 3], vec![7.]).as_deref(),
            Some("3 while still")
        );
        assert_eq!(report(vec![], vec![]), None);
    }
}
