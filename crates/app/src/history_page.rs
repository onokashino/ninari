//! The watch history: episodes by day with how far each got, to play again or remove; a removed one
//! can be brought back for a few seconds.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AnchoredPositionMode, AnyElement, Context, EventEmitter, FontWeight, IntoElement, Render, Task,
    Window, anchored, deferred, div, point, prelude::*, px, rgb, rgba,
};
use ninari_core::feeds::{self, History};
use ninari_core::library::{Changes, EpisodeProgress};

use crate::assets::icon;
use crate::button::{Size, Variant, button};
use crate::cards::poster_frame;
use crate::clock::{local_day, local_time, now, utc_offset};
use crate::core::Core;
use crate::data::{Load, fetch, title};
use crate::format::remaining;
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
}

impl EventEmitter<PageEvent> for HistoryPage {}

impl HistoryPage {
    pub fn new(core: Arc<Core>, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            core,
            list: Load::Pending,
            removed: None,
            error: None,
            timer: None,
        };
        page.reload(cx);
        page
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        fetch(
            cx,
            self.core
                .run(|c| async move { feeds::history(&c.library, &c.catalog, &c.posters) }),
            |page, list| page.list = list,
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
                let days = history_days(
                    &h.episodes,
                    |p| p.updated_at,
                    now(),
                    |t| local_day(t, utc_offset(t)),
                );
                for day in days {
                    let rows: Vec<AnyElement> = day
                        .items
                        .iter()
                        .filter_map(|p| {
                            let card = h.titles.iter().find(|t| t.anime.id == p.shikimori_id)?;
                            let name = title(&card.anime);
                            let (id, episode) = (p.shikimori_id, p.episode);
                            let request = Request {
                                id,
                                episode,
                                dub_key: p.dub_key.clone(),
                                start: (!p.watched && p.position_sec > 5.)
                                    .then(|| (p.position_sec - 5.).round()),
                                title: name.clone().into(),
                                palette: poster(card, cx).map(|(_, pal)| pal).unwrap_or_default(),
                            };
                            let row = (*p).clone();
                            let forget_name = name.clone();
                            Some(
                                div()
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
                                            .id((
                                                "history-title",
                                                (id as u64) << 16 | episode as u64,
                                            ))
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
                                                            .group_hover(
                                                                "history-title",
                                                                move |s| {
                                                                    s.text_color(theme::mix(
                                                                        rgb(theme::TEXT),
                                                                        accent,
                                                                        0.45,
                                                                    ))
                                                                },
                                                            )
                                                            .child(name),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .flex_wrap()
                                                            .gap_x(px(12.))
                                                            .text_color(rgb(theme::TEXT_2))
                                                            .text_size(px(13.))
                                                            .child(format!("Серия {episode}"))
                                                            .child(
                                                                div()
                                                                    .text_color(if p.watched {
                                                                        theme::mix(
                                                                            rgb(theme::TEXT),
                                                                            accent,
                                                                            0.7,
                                                                        )
                                                                    } else {
                                                                        rgb(theme::TEXT_3).into()
                                                                    })
                                                                    .child(episode_state(p)),
                                                            ),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .gap(px(4.))
                                            .child(
                                                round_action(
                                                    ("replay", (id as u64) << 16 | episode as u64),
                                                    "play",
                                                )
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    cx.emit(PageEvent::Play(request.clone()))
                                                })),
                                            )
                                            .child(
                                                round_action(
                                                    ("forget", (id as u64) << 16 | episode as u64),
                                                    "close",
                                                )
                                                .on_click(cx.listener(move |page, _, _, cx| {
                                                    page.remove(&row, forget_name.clone(), cx)
                                                })),
                                            ),
                                    )
                                    .into_any_element(),
                            )
                        })
                        .collect();
                    sections.push(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .max_w(px(880.))
                            .child(
                                div()
                                    .text_color(rgb(theme::TEXT_2))
                                    .font_weight(FontWeight::EXTRA_BOLD)
                                    .child(day.label),
                            )
                            .child(div().flex().flex_col().gap(px(8.)).children(rows))
                            .into_any_element(),
                    );
                }
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
