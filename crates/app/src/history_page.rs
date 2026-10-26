//! The watch history: episodes by day with how far each got, to play again or remove; a removed one
//! can be brought back for a few seconds.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AnchoredPositionMode, AnyElement, Context, EventEmitter, FontWeight, IntoElement, Render, Task,
    Window, anchored, deferred, div, point, prelude::*, px, rgb, rgba,
};
use ninari_core::feeds::{self, AnimeCard, History};
use ninari_core::library::{Changes, EpisodeProgress};

use crate::assets::icon;
use crate::button::{Size, Variant, button};
use crate::cards::poster_frame;
use crate::clock::{local_day, local_time, now, utc_offset};
use crate::core::Core;
use crate::data::{Load, fetch, title};
use crate::format::{plural, remaining};
use crate::lazy_grid::{self, GridTop, lazy_list};
use crate::library_page::content_width;
use crate::nav::{PageEvent, Route};
use crate::player::Request;
use crate::posters::poster;
use crate::rail;
use crate::schedule::history_days;
use crate::theme::{self, Accent};

const UNDO_SHOWN: Duration = Duration::from_secs(6);

/// How far an episode got: "Просмотрена", "осталось 12 мин" or "Только начата".
pub fn episode_state(p: &EpisodeProgress) -> String {
    if p.watched {
        "Просмотрена".into()
    } else if p.duration_sec <= 0. || p.position_sec < 30. {
        "Только начата".into()
    } else {
        remaining(p.position_sec, p.duration_sec)
    }
}

pub struct HistoryPage {
    core: Arc<Core>,
    list: Load<History>,
    /// The row just removed and its title, to bring back.
    removed: Option<(EpisodeProgress, String, Instant)>,
    error: Option<String>,
    timer: Option<Task<()>>,
    /// Counts the loads, so the days are split again only after one.
    generation: u64,
    /// The load and the local day the days were split for, and each day with its episodes.
    days: Option<(u64, i64, Vec<(String, Range<usize>)>)>,
    top: GridTop,
}

/// A line of the list: a day's heading, or episodes side by side.
enum Line {
    Day(String),
    /// Rows side by side, each the run of one title's episodes watched in a row.
    Rows(Vec<Range<usize>>),
}

/// Runs of the same title in a row: one line for "Серии 2–10" rather than nine.
fn runs(ids: &[i64]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        match out.last_mut() {
            Some(r) if ids[r.start] == *id => r.end = i + 1,
            _ => out.push(i..i + 1),
        }
    }
    out
}

const ROW_H: f32 = 88.;
const DAY_H: f32 = 44.;
const GAP: f32 = 8.;
/// The narrowest a row gets before the rows go fewer to a line.
const ROW_MIN: f32 = 520.;

impl EventEmitter<PageEvent> for HistoryPage {}

impl HistoryPage {
    pub fn new(core: Arc<Core>, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            core,
            list: Load::Pending,
            removed: None,
            error: None,
            timer: None,
            generation: 0,
            days: None,
            top: GridTop::default(),
        };
        page.reload(cx);
        page
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        fetch(
            cx,
            self.core
                .run(|c| async move { feeds::history(&c.library, &c.catalog, &c.posters) }),
            |page, list| {
                page.list = list;
                page.generation += 1;
            },
        );
    }

    fn remove(&mut self, p: &EpisodeProgress, name: String, cx: &mut Context<Self>) {
        let (id, episode) = (p.shikimori_id, p.episode);
        let job = self
            .core
            .run(move |c| async move { c.library.forget_episode(id, episode) });
        cx.spawn(async move |page, cx| {
            let result = job.await;
            page.update(cx, |page, cx| {
                match result {
                    Ok(Some(row)) => {
                        page.removed = Some((row, name, Instant::now()));
                        page.timer = Some(cx.spawn(async move |page, cx| {
                            cx.background_executor().timer(UNDO_SHOWN).await;
                            page.update(cx, |p, cx| {
                                p.removed = None;
                                cx.notify();
                            })
                            .ok();
                        }));
                    }
                    Ok(None) => {}
                    Err(e) => page.error = Some(format!("{e:#}")),
                }
                page.reload(cx);
                cx.emit(PageEvent::Changed);
            })
            .ok();
        })
        .detach();
    }

    fn restore(&mut self, cx: &mut Context<Self>) {
        let Some((row, ..)) = self.removed.take() else {
            return;
        };
        let job = self.core.run(move |c| async move {
            c.library.apply(&Changes {
                entries: vec![],
                progress: vec![row],
            })
        });
        cx.spawn(async move |page, cx| {
            let result = job.await;
            page.update(cx, |page, cx| {
                if let Err(e) = result {
                    page.error = Some(format!("{e:#}"));
                }
                page.reload(cx);
                cx.emit(PageEvent::Changed);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

fn round_action(id: impl Into<gpui::ElementId>, glyph: &str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .cursor_pointer()
        .size(px(36.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .group("action")
        .hover(|s| s.bg(theme::glass_strong()))
        .child(
            icon(glyph, 18.)
                .text_color(rgb(theme::TEXT_3))
                .group_hover("action", |s| s.text_color(rgb(theme::TEXT))),
        )
}

impl HistoryPage {
    /// One episode: when, the title, how far it got, play again and remove.
    fn row(
        &self,
        run: &[EpisodeProgress],
        titles: &HashMap<i64, &AnimeCard>,
        accent: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        // The newest first: its time, and the episode Play starts.
        let p = run.first()?;
        let card = *titles.get(&p.shikimori_id)?;
        let (low, high) = run.iter().fold((i64::MAX, 0), |(lo, hi), e| {
            (lo.min(e.episode), hi.max(e.episode))
        });
        let label = if run.len() > 1 {
            format!("Серии {low}–{high}")
        } else {
            format!("Серия {}", p.episode)
        };
        let state = if run.len() > 1 {
            let n = run.len() as i64;
            let watched = run.iter().filter(|e| e.watched).count() as i64;
            if watched == n {
                format!(
                    "{n} {}, все просмотрены",
                    plural(n, "серия", "серии", "серий")
                )
            } else {
                format!("просмотрено {watched} из {n}")
            }
        } else {
            episode_state(p)
        };
        let all_watched = run.iter().all(|e| e.watched);
        let single = run.len() == 1;
        let name = title(&card.anime);
        let (id, episode) = (p.shikimori_id, p.episode);
        let request = Request {
            id,
            episode,
            dub_key: p.dub_key.clone(),
            start: (!p.watched && p.position_sec > 5.).then(|| (p.position_sec - 5.).round()),
            title: name.clone().into(),
            palette: poster(card, cx).map(|(_, pal)| pal).unwrap_or_default(),
        };
        let row = (*p).clone();
        let forget_name = name.clone();
        Some(
            div()
                .h(px(ROW_H))
                .flex()
                .items_center()
                .gap(px(16.))
                .pl(px(14.))
                .pr(px(12.))
                .py(px(10.))
                .rounded(theme::R_L)
                .bg(theme::glass())
                .border_1()
                .border_color(theme::line())
                .hover(|s| s.bg(theme::glass_strong()))
                .child(
                    div()
                        .w(px(54.))
                        .flex_none()
                        .text_color(rgb(theme::TEXT_2))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .child(local_time(p.updated_at)),
                )
                .child(
                    div()
                        .id(("history-title", (id as u64) << 16 | episode as u64))
                        .group("history-title")
                        .cursor_pointer()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .items_center()
                        .gap(px(16.))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(PageEvent::Open(Route::Title(id)))
                        }))
                        .child(poster_frame(card, px(44.), px(8.), cx))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.))
                                .min_w(px(0.))
                                .child(
                                    div()
                                        .font_weight(FontWeight::BOLD)
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .group_hover("history-title", move |s| {
                                            s.text_color(theme::mix(rgb(theme::TEXT), accent, 0.45))
                                        })
                                        .child(name),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .gap_x(px(12.))
                                        .text_color(rgb(theme::TEXT_2))
                                        .text_size(px(13.))
                                        .child(label)
                                        .child(
                                            div()
                                                .text_color(if all_watched {
                                                    theme::mix(rgb(theme::TEXT), accent, 0.7)
                                                } else {
                                                    rgb(theme::TEXT_3).into()
                                                })
                                                .child(state),
                                        ),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(4.))
                        .child(
                            round_action(("replay", (id as u64) << 16 | episode as u64), "play")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(PageEvent::Play(request.clone()))
                                })),
                        )
                        // One button taking a run of episodes away at once is too easy to hit.
                        .when(single, |d| {
                            d.child(
                                round_action(
                                    ("forget", (id as u64) << 16 | episode as u64),
                                    "close",
                                )
                                .on_click(cx.listener(
                                    move |page, _, _, cx| {
                                        page.remove(&row, forget_name.clone(), cx)
                                    },
                                )),
                            )
                        }),
                )
                .into_any_element(),
        )
    }
}

impl Render for HistoryPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        let mut sections: Vec<AnyElement> = Vec::new();
        match &self.list {
            Load::Pending => sections.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .max_w(px(880.))
                    .children((0..5).map(|_| div().h(px(84.)).rounded(px(16.)).bg(theme::glass())))
                    .into_any_element(),
            ),
            Load::Failed(e) => sections.push(
                div()
                    .text_color(rgb(theme::DANGER))
                    .child(e.clone())
                    .into_any_element(),
            ),
            Load::Ready(h) if h.episodes.is_empty() => sections.push(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(16.))
                    .py(px(48.))
                    .child(
                        div()
                            .text_size(px(24.))
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child("Пока пусто"),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_2))
                            .child("Здесь появятся серии, как только ты что-нибудь посмотришь."),
                    )
                    .child(
                        button(
                            "to-catalog",
                            "В каталог",
                            Some("compass"),
                            Variant::Primary,
                            Size::M,
                            accent,
                        )
                        .on_click(
                            cx.listener(|_, _, _, cx| cx.emit(PageEvent::Open(Route::Catalog))),
                        ),
                    )
                    .into_any_element(),
            ),
            Load::Ready(h) => {
                let today = local_day(now(), utc_offset(now()));
                if self
                    .days
                    .as_ref()
                    .is_none_or(|(g, d, _)| (*g, *d) != (self.generation, today))
                {
                    // Splitting four hundred episodes into local days is not done on every frame.
                    let days = history_days(
                        &h.episodes,
                        |p| p.updated_at,
                        now(),
                        |t| local_day(t, utc_offset(t)),
                    );
                    let mut at = 0;
                    let ranges = days
                        .into_iter()
                        .map(|d| {
                            at += d.items.len();
                            (d.label, at - d.items.len()..at)
                        })
                        .collect();
                    self.days = Some((self.generation, today, ranges));
                }
                let titles: HashMap<i64, &AnimeCard> =
                    h.titles.iter().map(|t| (t.anime.id, t)).collect();
                // Rows side by side on a wide window, each at least ROW_MIN wide.
                let width = content_width(window);
                let columns = ((width + GAP) / (ROW_MIN + GAP)).floor().max(1.) as usize;
                let mut lines: Vec<Line> = Vec::new();
                for (label, range) in self.days.as_ref().map(|d| &d.2[..]).unwrap_or_default() {
                    lines.push(Line::Day(label.clone()));
                    let ids: Vec<i64> = h.episodes[range.clone()]
                        .iter()
                        .map(|p| p.shikimori_id)
                        .collect();
                    let groups: Vec<Range<usize>> = runs(&ids)
                        .into_iter()
                        .map(|r| r.start + range.start..r.end + range.start)
                        .collect();
                    for chunk in groups.chunks(columns) {
                        lines.push(Line::Rows(chunk.to_vec()));
                    }
                }
                let heights: Vec<f32> = lines
                    .iter()
                    .map(|l| match l {
                        Line::Day(_) => DAY_H,
                        Line::Rows(_) => ROW_H + GAP,
                    })
                    .collect();
                let screen = lazy_grid::screen(window);
                let top = self.top.clone();
                let list = lazy_list(&top, &heights, screen, |i| match &lines[i] {
                    Line::Day(label) => div()
                        .pt(px(12.))
                        .text_color(rgb(theme::TEXT_2))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .child(label.clone())
                        .into_any_element(),
                    Line::Rows(groups) => div()
                        .flex()
                        .gap(px(GAP))
                        .children(groups.iter().map(|g| {
                            div().flex_1().min_w(px(0.)).children(self.row(
                                &h.episodes[g.clone()],
                                &titles,
                                accent,
                                cx,
                            ))
                        }))
                        .children((groups.len()..columns).map(|_| div().flex_1()))
                        .into_any_element(),
                });
                sections.push(list);
            }
        }

        let undo = self
            .removed
            .as_ref()
            .filter(|(.., at)| at.elapsed() < UNDO_SHOWN)
            .map(|(row, name, _)| {
                let view = window.viewport_size();
                let bar = div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .pl(px(20.))
                    .pr(px(8.))
                    .py(px(8.))
                    .rounded_full()
                    .bg(rgba(0x1f2230f2))
                    .border_1()
                    .border_color(theme::line())
                    .text_size(px(13.))
                    .child(format!("Серия {} «{name}» удалена из истории", row.episode))
                    .child(
                        button("undo", "Вернуть", None, Variant::Quiet, Size::M, accent)
                            .on_click(cx.listener(|page, _, _, cx| page.restore(cx))),
                    );
                deferred(
                    anchored()
                        .position_mode(AnchoredPositionMode::Window)
                        .position(point(px(rail::WIDTH + 48.), view.height - px(80.)))
                        .child(bar),
                )
            });

        div()
            .flex()
            .flex_col()
            .gap(px(32.))
            .px(px(48.))
            .pt(px(50.))
            .pb(px(72.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(34.))
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child("История"),
                    )
                    .child(div().text_color(rgb(theme::TEXT_2)).child(
                        "Серии, которые ты открывал, по дням. Удалённая серия забывает, где остановилась и что была просмотрена.",
                    )),
            )
            .children(self.error.clone().map(|e| div().text_color(rgb(theme::DANGER)).child(e)))
            .children(sections)
            .children(undo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_watched_in_a_row_is_one_line() {
        assert_eq!(runs(&[7, 7, 7, 3, 7]), vec![0..3, 3..4, 4..5]);
        assert_eq!(runs(&[]), Vec::<Range<usize>>::new());
    }
}
