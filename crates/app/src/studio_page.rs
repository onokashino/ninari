//! A studio's page: everything it made, best rated or newest first, with how many and how good.

use std::sync::Arc;

use gpui::{
    AnyElement, Context, EventEmitter, FontWeight, IntoElement, Render, Window, div, prelude::*,
    px, rgb,
};
use ninari_core::catalog::browse::BrowseFilter;
use ninari_core::feeds::{self, AnimeCard};

use crate::cards::{CardText, poster_card_sized};
use crate::core::Core;
use crate::data::{Load, fetch};
use crate::episode_grid::chip;
use crate::lazy_grid::{self, GridTop, lazy_grid, row_height};
use crate::library_page::{content_width, grid_columns};
use crate::nav::{PageEvent, Route};
use crate::theme::{self, Accent};

/// Pages of the catalog asked for: a studio rarely has more than a few hundred titles.
const PAGES: u32 = 8;

#[derive(Clone, Copy, PartialEq)]
enum Order {
    Score,
    Newest,
}

pub struct StudioPage {
    id: i64,
    list: Load<Vec<AnimeCard>>,
    order: Order,
    grid_top: GridTop,
}

impl EventEmitter<PageEvent> for StudioPage {}

impl StudioPage {
    pub fn new(core: Arc<Core>, id: i64, cx: &mut Context<Self>) -> Self {
        fetch(
            cx,
            core.run(move |c| async move {
                let filter = BrowseFilter {
                    studio: Some(id),
                    order: Some("ranked".into()),
                    ..Default::default()
                };
                let mut all = Vec::new();
                for page in 1..=PAGES {
                    let got = c.catalog.browse(&filter, page).await?;
                    let last = got.len() < ninari_core::catalog::browse::PAGE_SIZE as usize;
                    all.extend(got.into_iter().map(|a| feeds::card(&c.posters, a)));
                    if last {
                        break;
                    }
                }
                Ok(all)
            }),
            |page: &mut Self, list| page.list = list,
        );
        Self {
            id,
            list: Load::Pending,
            order: Order::Score,
            grid_top: GridTop::default(),
        }
    }
}

impl Render for StudioPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        let width = content_width(window);
        let (columns, card_w) = grid_columns(width);
        let list = match &self.list {
            Load::Ready(list) => &list[..],
            _ => &[],
        };
        let name = list
            .iter()
            .find_map(|c| {
                let a = &c.anime;
                let i = a.studio_ids.iter().position(|s| *s == self.id)?;
                a.studios.get(i).cloned()
            })
            .unwrap_or_else(|| "Студия".into());
        let scored: Vec<f64> = list
            .iter()
            .filter_map(|c| c.anime.score)
            .filter(|s| *s > 0.)
            .collect();
        let average =
            (!scored.is_empty()).then(|| scored.iter().sum::<f64>() / scored.len() as f64);
        let mut shown: Vec<&AnimeCard> = list.iter().collect();
        if self.order == Order::Newest {
            shown.sort_by(|a, b| b.anime.year.cmp(&a.anime.year));
        }
        let facts = [
            (!list.is_empty()).then(|| {
                let n = list.len() as i64;
                format!(
                    "{n} {}",
                    crate::format::plural(n, "тайтл", "тайтла", "тайтлов")
                )
            }),
            average.map(|s| format!("средняя оценка ★ {}", format!("{s:.2}").replace('.', ","))),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let orders = [(Order::Score, "Лучшие"), (Order::Newest, "Новые")].map(|(o, label)| {
            chip(("studio-order", o as u64), self.order == o, accent)
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.order = o;
                    cx.notify();
                }))
                .child(label)
        });
        let body: AnyElement = match &self.list {
            Load::Pending => div()
                .text_color(rgb(theme::TEXT_3))
                .child("Собираю всё, что сделала студия…")
                .into_any_element(),
            Load::Failed(e) => div()
                .text_color(rgb(theme::DANGER))
                .child(e.clone())
                .into_any_element(),
            Load::Ready(_) => {
                let screen = lazy_grid::screen(window);
                let top = self.grid_top.clone();
                lazy_grid(
                    &top,
                    shown.len(),
                    columns,
                    row_height(card_w),
                    screen,
                    |i| {
                        let c = shown[i];
                        let id = c.anime.id;
                        let open = Box::new(cx.listener(move |_, _, _, cx| {
                            cx.emit(PageEvent::Open(Route::Title(id)))
                        }));
                        poster_card_sized(
                            c,
                            "studio",
                            i,
                            CardText::default(),
                            card_w,
                            accent,
                            open,
                            window,
                            cx,
                        )
                        .into_any_element()
                    },
                )
            }
        };
        div()
            .flex()
            .flex_col()
            .gap(px(28.))
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
                            .child(name),
                    )
                    .child(div().text_color(rgb(theme::TEXT_2)).child(facts)),
            )
            .child(div().flex().gap(px(8.)).children(orders))
            .child(body)
    }
}
