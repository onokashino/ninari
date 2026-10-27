//! The week's schedule: a day at a time, everything or only the user's titles, today's aired
//! episodes stepping back under a "Сейчас" line.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, EventEmitter, FontWeight, IntoElement, Render, Window, div, prelude::*,
    px, rgb,
};
use ninari_core::catalog::calendar::unix_of;
use ninari_core::feeds::{self, ScheduleItem};
use ninari_core::pace;

use crate::cards::poster_frame;
use crate::clock::{is_followed, local_day, local_time, now, utc_offset};
use crate::core::Core;
use crate::data::{Load, fetch, title};
use crate::episode_grid::chip;
use crate::format::{library_status, plural};
use crate::motion::rise;
use crate::nav::{PageEvent, Route};
use crate::schedule::{episode_line, schedule_days};
use crate::scroll_row::Row;
use crate::theme::{self, Accent};

pub struct SchedulePage {
    core: Arc<Core>,
    list: Load<Vec<ScheduleItem>>,
    /// Which of the seven days is on screen.
    day: usize,
    mine: bool,
    days_row: Row,
    /// The teams carrying each title: name, episodes out, usual delay in hours.
    dubs: HashMap<i64, Vec<(String, i64, Option<i64>)>>,
}

impl EventEmitter<PageEvent> for SchedulePage {}

impl SchedulePage {
    pub fn new(core: Arc<Core>, cx: &mut Context<Self>) -> Self {
        let page = Self {
            core,
            list: Load::Pending,
            day: 0,
            mine: false,
            days_row: Row::default(),
            dubs: HashMap::new(),
        };
        fetch(
            cx,
            page.core.run(|c| async move {
                let list = feeds::schedule(&c.schedule, &c.catalog, &c.library, &c.posters).await?;
                let ids: Vec<i64> = list.iter().map(|it| it.card.anime.id).collect();
                let seen = pace::seen(&c.db, &ids)?;
                let dubs = list
                    .iter()
                    .filter_map(|it| {
                        let rows = seen.get(&it.card.anime.id)?;
                        let paces = pace::pace(rows, it.episode, unix_of(&it.at)?)
                            .into_iter()
                            .map(|p| (c.resolver.dub_name(&p.key), p.episodes, p.lag_hours))
                            .collect();
                        Some((it.card.anime.id, paces))
                    })
                    .collect::<HashMap<_, _>>();
                Ok((list, dubs))
            }),
            |page, got| match got {
                Load::Ready((list, dubs)) => {
                    page.list = Load::Ready(list);
                    page.dubs = dubs;
                }
                Load::Failed(e) => page.list = Load::Failed(e),
                Load::Pending => {}
            },
        );
        page
    }
}

fn at(it: &ScheduleItem) -> Option<i64> {
    unix_of(&it.at)
}

fn local(t: i64) -> i64 {
    local_day(t, utc_offset(t))
}

impl Render for SchedulePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        let now = now();
        let items: Vec<ScheduleItem> = match &self.list {
            Load::Ready(list) => list
                .iter()
                .filter(|it| !self.mine || is_followed(it.status.as_deref()))
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        let mine_count = match &self.list {
            Load::Ready(list) => list
                .iter()
                .filter(|it| is_followed(it.status.as_deref()))
                .count(),
            _ => 0,
        };
        let days = schedule_days(&items, at, now, local);
        let current = self.day.min(days.len() - 1);

        let tabs: Vec<AnyElement> = days
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let picked = i == current;
                let n = d.items.len() as i64;
                let count = if n > 0 {
                    format!("{n} {}", plural(n, "серия", "серии", "серий"))
                } else {
                    "нет серий".into()
                };
                div()
                    .id(("day", i))
                    .cursor_pointer()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .min_w(px(112.))
                    .px(px(16.))
                    .py(px(10.))
                    .rounded(theme::R_M)
                    .border_1()
                    .map(|t| {
                        if picked {
                            t.bg(accent.opacity(0.2)).border_color(accent.opacity(0.45))
                        } else {
                            t.bg(theme::glass())
                                .border_color(theme::line())
                                .hover(|s| s.bg(theme::glass_strong()))
                        }
                    })
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.day = i;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .whitespace_nowrap()
                            .text_color(rgb(if picked { theme::TEXT } else { theme::TEXT_2 }))
                            .child(d.label.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .whitespace_nowrap()
                            .text_color(if picked {
                                theme::mix(rgb(theme::TEXT), accent, 0.7)
                            } else {
                                rgb(theme::TEXT_3).into()
                            })
                            .child(count),
                    )
                    .into_any_element()
            })
            .collect();
        let days_row =
            self.days_row
                .render("schedule-days", px(8.), (0., 0., 0.), tabs, window, cx);
        let mine = self.mine;
        let bar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .child(div().flex_1().min_w(px(0.)).child(days_row))
            .child(
                chip("mine", mine, accent)
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.mine = !page.mine;
                        cx.notify();
                    }))
                    .child("Только мои")
                    .when(mine_count > 0, |c| {
                        c.child(
                            div()
                                .ml(px(2.))
                                .px(px(7.))
                                .py(px(1.))
                                .rounded_full()
                                .bg(theme::glass_strong())
                                .text_size(px(12.))
                                .child(mine_count.to_string()),
                        )
                    }),
            );

        let body: AnyElement = match &self.list {
            Load::Pending => div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .max_w(px(880.))
                .children((0..5).map(|_| div().h(px(92.)).rounded(px(16.)).bg(theme::glass())))
                .into_any_element(),
            Load::Failed(e) => div()
                .text_color(rgb(theme::DANGER))
                .child(e.clone())
                .into_any_element(),
            Load::Ready(_) if days[current].items.is_empty() => div()
                .text_color(rgb(theme::TEXT_2))
                .child(if self.mine {
                    "Из твоих тайтлов в этот день ничего не выходит."
                } else {
                    "В этот день новых серий нет."
                })
                .into_any_element(),
            Load::Ready(_) => {
                let day = &days[current];
                let today = current == 0;
                let first_upcoming = if today {
                    day.items
                        .iter()
                        .position(|it| at(it).is_some_and(|t| t > now))
                } else {
                    None
                };
                // Today, episodes before the first upcoming one have aired (all of them if none is).
                let aired = |i: usize| today && first_upcoming.is_none_or(|f| i < f);
                let mut rows: Vec<AnyElement> = Vec::new();
                for (i, it) in day.items.iter().enumerate() {
                    if first_upcoming == Some(i) && i > 0 {
                        rows.push(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(12.))
                                .my(px(8.))
                                .text_color(accent)
                                .text_size(px(12.))
                                .font_weight(FontWeight::EXTRA_BOLD)
                                .child(div().w(px(14.)).h(px(1.)).bg(accent.opacity(0.6)))
                                .child("Сейчас")
                                .child(div().flex_1().h(px(1.)).bg(accent.opacity(0.6)))
                                .into_any_element(),
                        );
                    }
                    let id = it.card.anime.id;
                    let name = title(&it.card.anime);
                    let original = (it.card.anime.name != name).then(|| it.card.anime.name.clone());
                    let watching = it.status.as_deref() == Some("watching");
                    let row = div()
                        .id(("schedule-row", (id as u64) << 16 | it.episode as u64))
                        .group("schedule-row")
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap(px(16.))
                        .pl(px(14.))
                        .pr(px(18.))
                        .py(px(10.))
                        .rounded(theme::R_L)
                        .bg(theme::glass())
                        .border_1()
                        .border_color(theme::line())
                        .hover(|s| s.bg(theme::glass_strong()))
                        .when(aired(i), |r| r.opacity(0.5))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(PageEvent::Open(Route::Title(id)))
                        }))
                        .child(
                            div()
                                .w(px(62.))
                                .flex_none()
                                .text_size(px(18.))
                                .font_weight(FontWeight::EXTRA_BOLD)
                                .child(at(it).map(local_time).unwrap_or_default()),
                        )
                        .child(poster_frame(&it.card, px(52.), px(10.), cx))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .flex()
                                .flex_col()
                                .gap(px(3.))
                                .child(
                                    div()
                                        .font_weight(FontWeight::BOLD)
                                        .line_clamp(2)
                                        .text_ellipsis()
                                        .group_hover("schedule-row", move |s| {
                                            s.text_color(theme::mix(rgb(theme::TEXT), accent, 0.45))
                                        })
                                        .child(name),
                                )
                                .children(original.map(|o| {
                                    div()
                                        .text_color(rgb(theme::TEXT_3))
                                        .text_size(px(12.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(o)
                                }))
                                .child(
                                    div()
                                        .text_color(rgb(theme::TEXT_2))
                                        .text_size(px(13.))
                                        .child(episode_line(it.episode, it.card.anime.episodes)),
                                )
                                .children(
                                    self.dubs
                                        .get(&id)
                                        .map(|dubs| dub_line(dubs, it.episode, accent)),
                                ),
                        )
                        .children(it.status.as_deref().map(|s| {
                            div()
                                .flex_none()
                                .px(px(10.))
                                .py(px(4.))
                                .rounded_full()
                                .text_size(px(12.))
                                .font_weight(FontWeight::BOLD)
                                .whitespace_nowrap()
                                .map(|p| {
                                    if watching {
                                        p.bg(accent.opacity(0.22)).text_color(theme::mix(
                                            rgb(theme::TEXT),
                                            accent,
                                            0.8,
                                        ))
                                    } else {
                                        p.bg(theme::glass_strong()).text_color(rgb(theme::TEXT_2))
                                    }
                                })
                                .child(library_status(s))
                        }));
                    rows.push(
                        rise(
                            row,
                            (
                                "schedule-rise",
                                (current as u64) << 32 | (id as u64) << 8 | i as u64,
                            ),
                            i,
                            420,
                            28,
                            8.,
                        )
                        .into_any_element(),
                    );
                }
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .max_w(px(880.))
                    .children(rows)
                    .into_any_element()
            }
        };

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
                            .child("Расписание"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(10.))
                            .text_color(rgb(theme::TEXT_2))
                            .child(
                                div()
                                    .px(px(10.))
                                    .py(px(4.))
                                    .rounded_full()
                                    .bg(theme::glass_strong())
                                    .text_color(rgb(theme::TEXT))
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(13.))
                                    .child(format!("Твоё время, {}", zone_label(now))),
                            )
                            .child(
                                "Это когда серия выходит в Японии. Русская озвучка появляется \
                                 позже: обычно через несколько часов, иногда через день-два.",
                            ),
                    ),
            )
            .child(bar)
            .child(body)
    }
}

/// The user's time zone as "UTC+4" or "UTC+5:30".
/// The teams carrying the title, furthest first: those with this episode already lit, and how
/// long after Japan the quickest one usually comes.
fn dub_line(dubs: &[(String, i64, Option<i64>)], episode: i64, accent: gpui::Hsla) -> gpui::Div {
    const SHOWN: usize = 3;
    let quickest = dubs
        .iter()
        .filter_map(|(name, _, lag)| Some((lag.as_ref()?, name)))
        .min();
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.))
        .pt(px(2.))
        .text_size(px(12.))
        .children(dubs.iter().take(SHOWN).map(|(name, have, _)| {
            let out = *have >= episode;
            div()
                .px(px(8.))
                .py(px(2.))
                .rounded_full()
                .font_weight(FontWeight::SEMIBOLD)
                .map(|d| {
                    if out {
                        d.bg(accent.opacity(0.18)).text_color(rgb(theme::TEXT))
                    } else {
                        d.bg(theme::glass_strong()).text_color(rgb(theme::TEXT_2))
                    }
                })
                .child(format!("{name} · {have}"))
        }))
        .when(dubs.len() > SHOWN, |d| {
            d.child(
                div()
                    .text_color(rgb(theme::TEXT_3))
                    .child(format!("и ещё {}", dubs.len() - SHOWN)),
            )
        })
        .children(quickest.map(|(lag, name)| {
            div().text_color(rgb(theme::TEXT_3)).child(match lag {
                0 => format!("{name} обычно выходит в течение часа"),
                h if *h < 24 => format!("{name} обычно через ~{h} ч"),
                h => format!("{name} обычно через ~{} дн", (h + 12) / 24),
            })
        }))
}

fn zone_label(at: i64) -> String {
    let offset = utc_offset(at);
    let sign = if offset < 0 { '-' } else { '+' };
    let (hours, minutes) = (offset.abs() / 3600, offset.abs() % 3600 / 60);
    if minutes == 0 {
        format!("UTC{sign}{hours}")
    } else {
        format!("UTC{sign}{hours}:{minutes:02}")
    }
}
