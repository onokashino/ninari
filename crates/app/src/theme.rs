//! The app's design tokens: colours and sizes.

use std::time::{Duration, Instant};

use gpui::{App, Global, Hsla, Pixels, Rgba, Window, px, rgb, rgba};
use ninari_core::catalog::posters::{DEFAULT_ACCENT, DEFAULT_GLOW, Palette};

pub const INK: u32 = 0x101117;
pub const SURFACE: u32 = 0x171922;
pub const SURFACE_2: u32 = 0x1f2230;
pub const TEXT: u32 = 0xeeeae4;
pub const TEXT_2: u32 = 0xa7a3b0;
pub const TEXT_3: u32 = 0x6f6c7a;
pub const ACCENT_CONTRAST: u32 = 0x140f22;

pub fn line() -> Rgba {
    rgba(0xffffff14)
}

pub fn glass() -> Rgba {
    rgba(0xffffff0e)
}

pub fn glass_strong() -> Rgba {
    rgba(0xffffff17)
}

pub const DANGER: u32 = 0xff7a8a;
pub const OK: u32 = 0x7fe39a;
pub const WARN: u32 = 0xf0c867;

pub const R_M: Pixels = px(14.);
pub const R_L: Pixels = px(22.);

pub const FONT: &str = "Manrope";

/// `color-mix(a, b)` with `t` of `b`, mixed in sRGB.
pub fn mix(a: impl Into<Hsla>, b: impl Into<Hsla>, t: f32) -> Hsla {
    let (a, b) = (a.into().to_rgb(), b.into().to_rgb());
    Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
    .into()
}

/// "#b9a3ff" from a poster's palette; the default accent when it does not parse.
pub fn hex(color: &str) -> Hsla {
    u32::from_str_radix(color.trim_start_matches('#'), 16)
        .map(|v| rgb(v).into())
        .unwrap_or_else(|_| rgb(0xb9a3ff).into())
}

pub const T_FAST: Duration = Duration::from_millis(120);
pub const T_MED: Duration = Duration::from_millis(220);
/// `--t-slow`: how long the accent takes to follow a new title.
pub const T_SLOW: Duration = Duration::from_millis(700);

/// A CSS `cubic-bezier(x1, y1, x2, y2)` timing curve.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Curve(pub f32, pub f32, pub f32, pub f32);

/// `--ease`.
pub const EASE: Curve = Curve(0.2, 0.7, 0.2, 1.);
/// The poster cards' lift: it overshoots a little and settles.
pub const SPRING: Curve = Curve(0.22, 1.3, 0.36, 1.);

impl Curve {
    /// Progress at `t` in 0..=1; a curve may overshoot 1 on the way.
    pub fn at(self, t: f32) -> f32 {
        let Curve(x1, y1, x2, y2) = self;
        let bezier = |p1: f32, p2: f32, s: f32| {
            3. * p1 * s * (1. - s) * (1. - s) + 3. * p2 * s * s * (1. - s) + s * s * s
        };
        // x(s) only grows, so halving finds the s whose x is t.
        let (mut lo, mut hi) = (0f32, 1f32);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.;
            if bezier(x1, x2, mid) < t {
                lo = mid
            } else {
                hi = mid
            }
        }
        bezier(y1, y2, (lo + hi) / 2.)
    }
}

pub fn ease(t: f32) -> f32 {
    EASE.at(t)
}

/// The accent and glow of the title on screen, eased from the previous title's.
pub struct Accent {
    from: [Rgba; 2],
    to: [Rgba; 2],
    since: Instant,
}

impl Global for Accent {}

impl Accent {
    fn at(&self, now: Instant) -> [Rgba; 2] {
        let t = ease((now - self.since).as_secs_f32() / T_SLOW.as_secs_f32()).min(1.);
        let mix = |a: Rgba, b: Rgba| Rgba {
            r: a.r + (b.r - a.r) * t,
            g: a.g + (b.g - a.g) * t,
            b: a.b + (b.b - a.b) * t,
            a: a.a + (b.a - a.a) * t,
        };
        [mix(self.from[0], self.to[0]), mix(self.from[1], self.to[1])]
    }

    /// Heads for `palette` and keeps redrawing until it gets there. The root view calls this
    /// every frame; the first call takes the colours at once.
    pub fn follow(palette: &Palette, window: &mut Window, cx: &mut App) {
        let to = [hex(&palette.accent).to_rgb(), hex(&palette.glow).to_rgb()];
        let now = Instant::now();
        if !cx.has_global::<Accent>() {
            cx.set_global(Accent {
                from: to,
                to,
                since: now - T_SLOW,
            });
        }
        let accent = cx.global_mut::<Accent>();
        if accent.to != to {
            *accent = Accent {
                from: accent.at(now),
                to,
                since: now,
            };
        }
        if now - accent.since < T_SLOW {
            window.request_animation_frame();
        }
    }

    /// The accent and the glow this frame.
    pub fn colors(cx: &App) -> (Hsla, Hsla) {
        match cx.try_global::<Accent>() {
            Some(a) => {
                let [accent, glow] = a.at(Instant::now());
                (accent.into(), glow.into())
            }
            None => (hex(DEFAULT_ACCENT), hex(DEFAULT_GLOW)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SPRING, ease};

    #[test]
    fn ease_runs_from_start_to_end_and_front_loads() {
        assert!(ease(0.).abs() < 1e-4);
        assert!((ease(1.) - 1.).abs() < 1e-4);
        // Mostly there by the middle, as the CSS curve is.
        assert!(ease(0.5) > 0.85 && ease(0.5) < 0.95, "{}", ease(0.5));
        assert!((0..10).all(|i| ease(i as f32 / 10.) <= ease((i + 1) as f32 / 10.)));
    }

    #[test]
    fn the_spring_overshoots_then_settles() {
        assert!(SPRING.at(0.5) > 1.);
        assert!((SPRING.at(1.) - 1.).abs() < 1e-4);
    }
}
