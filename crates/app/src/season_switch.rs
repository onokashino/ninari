//! The timeline of a switch between seasons on the title page: the old season's content
//! fades out against the direction, waits for the new one if it is not in yet, and the new
//! one comes in from the side of the direction, its episode tiles as a wave.

/// The old content fading out.
pub const OUT_MS: u64 = 160;
/// The new content coming in.
pub const IN_MS: u64 = 360;
/// Between two tiles of the wave, and the most steps it takes.
pub const TILE_STEP_MS: u64 = 18;
const TILE_STEPS: u64 = 14;
/// How far the content moves out and in, in logical pixels.
pub const OUT_SHIFT: f32 = 18.;
pub const IN_SHIFT: f32 = 24.;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    /// The old season, faded this far (0..1).
    Out(f32),
    /// The old season faded out, the new one not in yet.
    Hold,
    /// The new season, in this far (0..1); `ms` since it started coming in.
    In {
        t: f32,
        ms: u64,
    },
    Done,
}

/// Where a switch is `elapsed` ms after it started; `ready` is when the new season's card was
/// in, in ms after the start (None while it is not).
pub fn phase(elapsed: u64, ready: Option<u64>) -> Phase {
    if elapsed < OUT_MS {
        return Phase::Out(elapsed as f32 / OUT_MS as f32);
    }
    let Some(ready) = ready else {
        return Phase::Hold;
    };
    let since = elapsed.saturating_sub(ready.max(OUT_MS));
    let wave = TILE_STEPS * TILE_STEP_MS;
    if since >= IN_MS + wave {
        Phase::Done
    } else {
        Phase::In {
            t: (since as f32 / IN_MS as f32).min(1.),
            ms: since,
        }
    }
}

/// How far in the `index`th tile is, `ms` after the new season started coming in.
pub fn tile_t(ms: u64, index: usize) -> f32 {
    let delay = (index as u64).min(TILE_STEPS) * TILE_STEP_MS;
    (ms.saturating_sub(delay) as f32 / IN_MS as f32).min(1.)
}

/// 1. when `to` comes after `from` in the franchise's order, -1. otherwise.
pub fn direction(order: &[i64], from: i64, to: i64) -> f32 {
    let at = |id| order.iter().position(|p| *p == id);
    match (at(from), at(to)) {
        (Some(a), Some(b)) if b < a => -1.,
        _ => 1.,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fades_out_first_then_waits_for_the_new_season() {
        assert_eq!(phase(0, Some(0)), Phase::Out(0.));
        assert_eq!(phase(80, None), Phase::Out(0.5));
        assert_eq!(phase(200, None), Phase::Hold);
    }

    #[test]
    fn comes_in_once_both_the_fade_and_the_card_are_done() {
        // Card in before the fade ended: in starts at the end of the fade.
        assert_eq!(phase(OUT_MS, Some(10)), Phase::In { t: 0., ms: 0 });
        assert_eq!(phase(OUT_MS + 180, Some(10)), Phase::In { t: 0.5, ms: 180 });
        // Card in late: in starts when it came.
        assert_eq!(
            phase(500, Some(400)),
            Phase::In {
                t: 100. / IN_MS as f32,
                ms: 100
            }
        );
        assert_eq!(
            phase(OUT_MS + IN_MS + 14 * TILE_STEP_MS, Some(0)),
            Phase::Done
        );
    }

    #[test]
    fn tiles_follow_one_another_up_to_a_cap() {
        assert_eq!(tile_t(0, 0), 0.);
        assert_eq!(tile_t(IN_MS, 0), 1.);
        assert_eq!(tile_t(TILE_STEP_MS, 1), 0.);
        assert_eq!(tile_t(1000, 40), tile_t(1000, 14));
    }

    #[test]
    fn direction_follows_the_franchise_order() {
        let order = [1, 2, 3];
        assert_eq!(direction(&order, 1, 3), 1.);
        assert_eq!(direction(&order, 3, 2), -1.);
        assert_eq!(direction(&order, 9, 2), 1.);
    }
}
