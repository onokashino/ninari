//! The statistics: watch time, episodes, finished titles and the streak, episodes per month,
//! favourite genres and studios, the library by status.

use std::sync::Arc;

use gpui::{
    AnyElement, Context, Div, EventEmitter, FontWeight, Hsla, IntoElement, Render, Window, div,
    linear_color_stop, linear_gradient, prelude::*, px, relative, rgb,
};
use ninari_core::feeds::{self, AnimeCard};
use ninari_core::stats::Stats;

use crate::cards::poster_frame;
use crate::clock::{now, utc_offset};
use crate::core::Core;
use crate::data::{Load, fetch, title};
use crate::format::{library_status, plural};
use crate::library_page::content_width;
use crate::nav::{PageEvent, Route};
use crate::theme::{self, Accent};

/// Watch time as a number and its unit: minutes under an hour, hours with a decimal under ten.
pub fn hours_label(seconds: i64) -> (String, &'static str) {
    if seconds < 3600 {
        return ((seconds / 60).to_string(), "мин");
    }
    let hours = seconds as f64 / 3600.;
    if hours < 10. {
        let rounded = (hours * 10.).round() / 10.;
        return (rounded.to_string().replace('.', ","), "ч");
    }
    (grouped(hours.round() as i64), "ч")
}

/// 1234 as "1 234", with a no-break space.
pub fn grouped(n: i64) -> String {
    let digits = n.abs().to_string();
    let mut out = String::new();
    for (i, d) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push('\u{a0}');
        }
        out.push(d);
    }
    if n < 0 { format!("-{out}") } else { out }
}

const MONTHS: [&str; 12] = [
    "янв", "фев", "мар", "апр", "май", "июн", "июл", "авг", "сен", "окт", "ноя", "дек",
];

/// "2026-09" as "сен".
pub fn month_label(month: &str) -> &str {
    month
        .get(5..7)
        .and_then(|m| m.parse::<usize>().ok())
        .and_then(|m| MONTHS.get(m.wrapping_sub(1)))
        .copied()
        .unwrap_or(month)
}

/// Parts of a whole with their fraction of it; empty parts are left out.
pub fn shares(parts: &[(String, i64)]) -> Vec<(&str, i64, f32)> {
    let total: i64 = parts.iter().map(|(_, n)| n).sum();
    if total == 0 {
        return Vec::new();
    }
    parts
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(key, n)| (key.as_str(), *n, *n as f32 / total as f32))
        .collect()
}

pub struct StatsPage {
    core: Arc<Core>,
    stats: Load<Stats>,
    /// The titles with the most episodes watched, with their cards for the posters.
    top: Vec<(AnimeCard, i64)>,
}

impl EventEmitter<PageEvent> for StatsPage {}

impl StatsPage {
    pub fn new(core: Arc<Core>, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            core,
            stats: Load::Pending,
            top: Vec::new(),
        };
        page.reload(cx);
        page
    }

    /// Counts again; watching changed the numbers. The old ones stay until the new are in.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let offset = utc_offset(now()) / 60;
        fetch(
            cx,
            self.core.run(move |c| async move {
                let stats = feeds::stats(&c.library, &c.catalog, offset)?;
                let ids: Vec<i64> = stats.top.iter().map(|(id, _)| *id).collect();
                let cards = c.catalog.cached_many(&ids)?;
                let top = stats
                    .top
                    .iter()
                    .filter_map(|(id, n)| {
                        let a = cards.iter().find(|a| a.id == *id)?.clone();
                        Some((feeds::card(&c.posters, a), *n))
                    })
                    .collect();
                Ok((stats, top))
            }),
            |page, got| match got {
                Load::Ready((stats, top)) => {
                    page.stats = Load::Ready(stats);
                    page.top = top;
                }
                Load::Failed(e) => page.stats = Load::Failed(e),
                Load::Pending => {}
            },
        );
    }
}

fn card() -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .p(px(24.))
        .min_w(px(0.))
        .rounded(theme::R_L)
        .bg(theme::glass())
        .border_1()
        .border_color(theme::line())
}

fn heading(text: &'static str) -> Div {
    div()
        .text_size(px(18.))
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(text)
}

fn figure(value: String, unit: Option<&'static str>, label: String) -> Div {
    card()
        .flex_1()
        .gap(px(4.))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(34.))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .child(value),
                )
                .children(unit.map(|u| {
                    div()
                        .text_size(px(18.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(theme::TEXT_2))
                        .child(u)
                })),
        )
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::TEXT_3))
                .child(label),
        )
}

/// A row per name, its bar as long as its share of the largest.
fn bars(items: &[(String, i64)], accent: Hsla) -> Div {
    let max = items.iter().map(|(_, n)| *n).max().unwrap_or(1).max(1);
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .text_size(px(13.))
        .children(items.iter().map(|(name, n)| {
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .w(px(144.))
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(name.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .h(px(8.))
                        .rounded_full()
                        .bg(theme::glass_strong())
                        .child(
                            div()
                                .h_full()
                                .w(relative(*n as f32 / max as f32))
                                .rounded_full()
                                .bg(accent),
                        ),
                )
                .child(
                    div()
                        .w(px(40.))
                        .flex_none()
                        .flex()
                        .justify_end()
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(theme::TEXT_3))
                        .child(n.to_string()),
                )
        }))
}

fn status_color(key: &str, accent: Hsla) -> Hsla {
    match key {
        "watching" => accent,
        "planned" => theme::mix(rgb(theme::TEXT_3), accent, 0.45),
        "completed" => rgb(theme::OK).into(),
        "on_hold" => rgb(theme::WARN).into(),
        _ => rgb(theme::DANGER).into(),
    }
}

impl Render for StatsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        let page = div()
            .flex()
            .flex_col()
            .gap(px(32.))
            .px(px(48.))
            .pt(px(50.))
            .pb(px(72.))
            .child(
                div()
                    .text_size(px(34.))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child("Статистика"),
            );
        let d = match &self.stats {
            Load::Pending => return page.child(card().h(px(120.))),
            Load::Failed(e) => {
                return page.child(div().text_color(rgb(theme::DANGER)).child(e.clone()));
            }
            Load::Ready(d) => d,
        };
        let parts = shares(&d.statuses);
        if d.episodes == 0 && parts.is_empty() {
            return page.child(
                div().max_w(px(560.)).text_color(rgb(theme::TEXT_2)).child(
                    "Пока нечего считать. Посмотри что-нибудь, и здесь появятся часы, серии и любимые жанры.",
                ),
            );
        }
        // Two figures to a row when the window is narrow, as the CSS has it under 900px.
        let narrow = content_width(window) < 900. - 96.;
        let (time, unit) = hours_label(d.seconds);
        let figures: Vec<AnyElement> = vec![
            figure(
                time,
                Some(unit),
                if d.estimated {
                    "за просмотром, примерно".into()
                } else {
                    "за просмотром".into()
                },
            ),
            figure(
                grouped(d.episodes),
                None,
                format!(
                    "{} просмотрено",
                    plural(d.episodes, "серия", "серии", "серий")
                ),
            ),
            figure(
                d.completed.to_string(),
                None,
                format!(
                    "{} досмотрено",
                    plural(d.completed, "тайтл", "тайтла", "тайтлов")
                ),
            ),
            figure(
                d.current_streak.to_string(),
                Some(plural(d.current_streak, "день", "дня", "дней")),
                format!("подряд сейчас, рекорд {}", d.best_streak),
            ),
        ]
        .into_iter()
        .map(|f| {
            f.min_w(px(if narrow { 260. } else { 0. }))
                .into_any_element()
        })
        .collect();

        let busiest = d.months.iter().map(|(_, n)| *n).max().unwrap_or(1).max(1);
        let months =
            card()
                .child(heading("Серии по месяцам"))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .children(d.months.iter().map(|(month, n)| {
                            let share = *n as f32 / busiest as f32;
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .w_full()
                                        .max_w(px(44.))
                                        .h(px(140.))
                                        .flex()
                                        .flex_col()
                                        .justify_end()
                                        .rounded(px(8.))
                                        .bg(theme::glass_strong())
                                        .child(
                                            div()
                                                .w_full()
                                                .h(relative(share))
                                                .min_h(px(3.))
                                                .rounded(px(8.))
                                                .bg(linear_gradient(
                                                    0.,
                                                    linear_color_stop(accent.opacity(0.7), 0.),
                                                    linear_color_stop(accent, 1.),
                                                )),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(rgb(theme::TEXT_3))
                                        .child(month_label(month).to_string()),
                                )
                        })),
                );

        let lists: Vec<Div> = [("Любимые жанры", &d.genres), ("Студии", &d.studios)]
            .into_iter()
            .filter(|(_, items)| !items.is_empty())
            .map(|(title, items)| {
                card()
                    .flex_1()
                    .child(heading(title))
                    .child(bars(items, accent))
            })
            .collect();

        let library = (!parts.is_empty()).then(|| {
            card()
                .child(heading("Библиотека"))
                .child(
                    div()
                        .flex()
                        .gap(px(2.))
                        .h(px(12.))
                        // GPUI clips square, so the ends are rounded on the parts themselves.
                        .children(parts.iter().enumerate().map(|(i, (key, _, share))| {
                            div()
                                .h_full()
                                .w(relative(*share))
                                .bg(status_color(key, accent))
                                .when(i == 0, |d| d.rounded_l_full())
                                .when(i + 1 == parts.len(), |d| d.rounded_r_full())
                        })),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_x(px(24.))
                        .gap_y(px(8.))
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .children(parts.iter().map(|(key, count, _)| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .size(px(10.))
                                        .rounded_full()
                                        .bg(status_color(key, accent)),
                                )
                                .child(library_status(key))
                                .child(
                                    div()
                                        .text_color(rgb(theme::TEXT_3))
                                        .child(count.to_string()),
                                )
                        })),
                )
        });

        let activity = self.activity(d, accent);
        let top = self.top_titles(accent, cx);
        page.child(div().flex().flex_wrap().gap(px(16.)).children(figures))
            .child(
                div()
                    .flex()
                    .when(narrow, |d| d.flex_col())
                    .gap(px(16.))
                    .child(activity)
                    .children(top),
            )
            .child(months)
            .when(!lists.is_empty(), |p| {
                p.child(
                    div()
                        .flex()
                        .when(narrow, |d| d.flex_col())
                        .gap(px(16.))
                        .children(lists),
                )
            })
            .children(library)
    }
}

impl StatsPage {
    /// Half a year of days as a grid of weeks, each day as bright as the episodes watched on it.
    fn activity(&self, d: &Stats, accent: gpui::Hsla) -> Div {
        const CELL: f32 = 13.;
        let max = d.activity.iter().copied().max().unwrap_or(0).max(1);
        let days_on = d.activity.iter().filter(|&&n| n > 0).count() as i64;
        let weeks = d.activity.chunks(7).map(|week| {
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .children(week.iter().map(|&n| {
                    let cell = div().size(px(CELL)).rounded(px(3.));
                    if n == 0 {
                        cell.bg(theme::glass_strong())
                    } else {
                        cell.bg(accent.opacity(0.3 + 0.7 * (n as f32 / max as f32)))
                    }
                }))
        });
        card()
            .flex_1()
            .child(heading("Активность за полгода"))
            .child(div().flex().gap(px(3.)).children(weeks))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(theme::TEXT_3))
                    .child(format!(
                        "{days_on} {} с просмотром, чем ярче день, тем больше серий",
                        plural(days_on, "день", "дня", "дней")
                    )),
            )
    }

    /// The titles watched the most, as posters.
    fn top_titles(&self, accent: gpui::Hsla, cx: &mut Context<Self>) -> Option<Div> {
        if self.top.is_empty() {
            return None;
        }
        let items = self.top.iter().map(|(c, n)| {
            let id = c.anime.id;
            div()
                .id(("stats-top", id as u64))
                .group("stats-top")
                .cursor_pointer()
                .w(px(96.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .on_click(
                    cx.listener(move |_, _, _, cx| cx.emit(PageEvent::Open(Route::Title(id)))),
                )
                .child(poster_frame(c, px(96.), theme::R_M, cx))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::BOLD)
                        .line_clamp(2)
                        .text_ellipsis()
                        .group_hover("stats-top", move |s| s.text_color(accent))
                        .child(title(&c.anime)),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(theme::TEXT_3))
                        .child(format!("{n} {}", plural(*n, "серия", "серии", "серий"))),
                )
        });
        Some(
            card()
                .flex_1()
                .child(heading("Больше всего серий"))
                .child(div().flex().flex_wrap().gap(px(16.)).children(items)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_time_reads_in_hours_or_minutes_when_short() {
        assert_eq!(hours_label(0), ("0".into(), "мин"));
        assert_eq!(hours_label(45 * 60), ("45".into(), "мин"));
        assert_eq!(hours_label(3 * 3600 + 1800), ("3,5".into(), "ч"));
        assert_eq!(hours_label(1234 * 3600), ("1\u{a0}234".into(), "ч"));
        assert_eq!(grouped(1_234_567), "1\u{a0}234\u{a0}567");
    }

    #[test]
    fn months_are_short_russian_names() {
        assert_eq!(month_label("2026-09"), "сен");
        assert_eq!(month_label("2026-01"), "янв");
        assert_eq!(month_label("junk"), "junk");
    }

    #[test]
    fn shares_skip_empty_parts() {
        let parts = [
            ("a".to_string(), 1),
            ("b".to_string(), 3),
            ("c".to_string(), 0),
        ];
        assert_eq!(shares(&parts), [("a", 1, 0.25), ("b", 3, 0.75)]);
    }
}
