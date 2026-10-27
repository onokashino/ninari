//! The "Дальше" card over the episode tiles: the episode to watch next, how far it got, or
//! that everything is watched or the next one is not out yet.

use std::rc::Rc;

use gpui::{AnyElement, App, FontWeight, Hsla, Window, div, prelude::*, px, rgb};
use ninari_core::feeds::ContinueAt;
use ninari_core::library::EpisodeProgress;

use crate::assets::icon;
use crate::theme;

pub type OnPlay = Rc<dyn Fn(i64, &mut Window, &mut App)>;

#[derive(Debug, Clone, PartialEq)]
pub enum NextUp {
    /// `started` when some of it was watched (`fraction` of it, 0..1); `first` when nothing
    /// of the title was watched yet.
    Play {
        episode: i64,
        fraction: f64,
        started: bool,
        first: bool,
    },
    /// Everything out has been watched and the title is finished.
    AllWatched,
    /// Everything available has been watched; more is coming.
    Waiting,
}

/// `cont` is the library's episode to continue, `in_dub` the episodes the chosen dub has,
/// `released` whether the title has finished airing.
pub fn next_up(
    cont: Option<&ContinueAt>,
    progress: &[EpisodeProgress],
    in_dub: &[i64],
    released: bool,
) -> NextUp {
    let Some(cont) = cont else {
        return if released {
            NextUp::AllWatched
        } else {
            NextUp::Waiting
        };
    };
    // The dub may not have the episode yet: offer the next one it has.
    let Some(episode) = in_dub.iter().copied().filter(|n| *n >= cont.episode).min() else {
        return NextUp::Waiting;
    };
    let p = progress.iter().find(|p| p.episode == episode);
    let fraction = match p {
        Some(p) if p.duration_sec > 0. && !p.watched => (p.position_sec / p.duration_sec).min(1.),
        _ => 0.,
    };
    NextUp::Play {
        episode,
        fraction,
        started: episode == cont.episode && cont.position > 0.,
        first: progress.is_empty(),
    }
}

/// The card: `title` is MyAnimeList's name of the episode, `left` "ещё 19 мин", `waiting` the
/// airing line ("Следующая серия ...") shown when the next one is not out.
pub fn next_card(
    next: &NextUp,
    title: Option<&str>,
    synopsis: Option<&str>,
    left: Option<String>,
    waiting: Option<String>,
    accent: Hsla,
    on_play: OnPlay,
) -> Option<AnyElement> {
    let (episode, head, line, fraction) = match next {
        NextUp::Play {
            episode,
            fraction,
            started,
            first,
        } => {
            let line = match title {
                Some(t) => format!("Серия {episode} · {t}"),
                None => format!("Серия {episode}"),
            };
            let head = if *first {
                "Начать"
            } else if *started {
                "Продолжить"
            } else {
                "Дальше"
            };
            (*episode, head, line, *fraction)
        }
        NextUp::AllWatched => (1, "Всё посмотрено", "Пересмотреть с 1 серии".into(), 0.),
        NextUp::Waiting => {
            return waiting.map(|w| {
                div()
                    .px(px(16.))
                    .py(px(12.))
                    .rounded(theme::R_M)
                    .bg(theme::glass())
                    .text_color(rgb(theme::TEXT_2))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(w)
                    .into_any_element()
            });
        }
    };
    const BAR: f32 = 220.;
    Some(
        div()
            .id("next-up")
            .cursor_pointer()
            .max_w(px(640.))
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(14.))
            .py(px(12.))
            .rounded(theme::R_L)
            .border_1()
            .border_color(accent.opacity(0.4))
            .bg(accent.opacity(0.14))
            .hover(|s| s.bg(accent.opacity(0.22)))
            .on_click(move |_, window, cx| on_play(episode, window, cx))
            .child(
                div()
                    .size(px(42.))
                    .flex_none()
                    .rounded_full()
                    .bg(accent)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("play", 18.).text_color(rgb(theme::ACCENT_CONTRAST))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(theme::TEXT_2))
                            .child(head),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(line),
                    )
                    .children(synopsis.map(|t| {
                        div()
                            .mt(px(2.))
                            .text_size(px(12.))
                            .line_height(px(17.))
                            .text_color(rgb(theme::TEXT_3))
                            .line_clamp(2)
                            .text_ellipsis()
                            .child(t.to_string())
                    }))
                    .when(fraction > 0., |d| {
                        d.child(
                            div()
                                .mt(px(6.))
                                .w(px(BAR))
                                .h(px(4.))
                                .rounded_full()
                                .bg(theme::glass_strong())
                                .child(
                                    div()
                                        .h_full()
                                        .w(px(BAR * fraction as f32))
                                        .rounded_full()
                                        .bg(accent),
                                ),
                        )
                    }),
            )
            .children(left.map(|l| {
                div()
                    .flex_none()
                    .text_size(px(13.))
                    .text_color(rgb(theme::TEXT_2))
                    .child(l)
            }))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prog(episode: i64, position_sec: f64, watched: bool) -> EpisodeProgress {
        EpisodeProgress {
            shikimori_id: 1,
            episode,
            position_sec,
            duration_sec: 1400.,
            watched,
            dub_key: None,
            extension_id: None,
            updated_at: episode,
        }
    }

    fn at(episode: i64, position: f64) -> ContinueAt {
        ContinueAt { episode, position }
    }

    #[test]
    fn nothing_watched_starts_with_the_first() {
        assert_eq!(
            next_up(Some(&at(1, 0.)), &[], &[1, 2, 3], true),
            NextUp::Play {
                episode: 1,
                fraction: 0.,
                started: false,
                first: true
            }
        );
    }

    #[test]
    fn a_started_episode_shows_how_far_it_got() {
        let progress = [prog(1, 1400., true), prog(2, 700., false)];
        assert_eq!(
            next_up(Some(&at(2, 690.)), &progress, &[1, 2, 3], true),
            NextUp::Play {
                episode: 2,
                fraction: 0.5,
                started: true,
                first: false
            }
        );
    }

    #[test]
    fn all_watched_or_waiting_when_there_is_nothing_to_continue() {
        assert_eq!(
            next_up(None, &[prog(12, 1400., true)], &[12], true),
            NextUp::AllWatched
        );
        assert_eq!(
            next_up(None, &[prog(7, 1400., true)], &[7], false),
            NextUp::Waiting
        );
    }

    #[test]
    fn the_dub_may_not_have_the_next_episode_yet() {
        let progress = [prog(4, 1400., true)];
        assert_eq!(
            next_up(Some(&at(5, 0.)), &progress, &[1, 2, 3, 4, 6], false),
            NextUp::Play {
                episode: 6,
                fraction: 0.,
                started: false,
                first: false
            }
        );
        assert_eq!(
            next_up(Some(&at(5, 0.)), &progress, &[1, 2, 3, 4], false),
            NextUp::Waiting
        );
    }
}
