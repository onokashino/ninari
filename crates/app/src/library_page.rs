//! The library: a tab per status, a search through it, an order, and the titles in a grid with how
//! much of each is watched.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, Entity, EventEmitter, FontWeight, IntoElement, Render, Window, div,
    prelude::*, px, rgb,
};
use ninari_core::feeds::{self, LibraryItem};

use crate::cards::{CardText, poster_card_sized};
use crate::core::Core;
use crate::data::{Load, fetch};
use crate::episode_grid::chip;
use crate::format::library_status;
use crate::lazy_grid::{self, GridTop, lazy_grid, row_height};
use crate::library::{SORTS, Sort, matches, query_variants, sort};
use crate::nav::Route;
use crate::popover::Menu;
use crate::rail;
use crate::select::{SelectOption, select};
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, Accent};

/// A page asks the window to go somewhere.
pub struct Open(pub Route);

const TABS: [&str; 5] = ["watching", "planned", "completed", "on_hold", "dropped"];

pub struct LibraryPage {
    core: Arc<Core>,
    grid_top: GridTop,
    status: &'static str,
    sort: Sort,
    sort_menu: Menu,
    query: Entity<TextInput>,
    lists: HashMap<&'static str, Load<Vec<LibraryItem>>>,
}

impl EventEmitter<Open> for LibraryPage {}

impl LibraryPage {
    pub fn new(core: Arc<Core>, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| {
            let mut input = TextInput::new("Найти в библиотеке", cx);
            input.compact = true;
            input
        });
        cx.subscribe(&query, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Changed) {
                cx.notify();
            }
        })
        .detach();
        let mut page = Self {
            core,
            grid_top: GridTop::default(),
            status: "watching",
            sort: Sort::Recent,
            sort_menu: Menu::default(),
            query,
            lists: HashMap::new(),
        };
        page.load(cx);
        page
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let status = self.status;
        self.lists.entry(status).or_insert(Load::Pending);
        fetch(
            cx,
            self.core.run(move |c| async move {
                feeds::library_items(&c.library, &c.catalog, &c.posters, Some(status)).await
            }),
            move |page, list| {
                page.lists.insert(status, list);
            },
        );
    }

    /// Watching or the lists changed elsewhere: the tab on screen is fetched again.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.lists.clear();
        self.load(cx);
    }

    fn open(&self, route: Route, cx: &mut Context<Self>) -> crate::OnClick {
        Box::new(cx.listener(move |_, _, _, cx| cx.emit(Open(route))))
    }
}

/// A quiet link beside a heading.
pub fn more_link(
    id: &'static str,
    label: &'static str,
    accent: gpui::Hsla,
    on_click: crate::OnClick,
) -> impl IntoElement {
    div()
        .id(id)
        .cursor_pointer()
        .text_size(px(13.))
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(theme::TEXT_3))
        .hover(move |s| s.text_color(accent))
        .on_click(on_click)
        .child(label)
}

/// The room a page's content has between its side paddings. Pages of cards take the whole
/// window, so a wide one gets more columns rather than an empty side.
pub fn content_width(window: &Window) -> f32 {
    f32::from(window.viewport_size().width - px(rail::WIDTH)) - 96.
}

/// Columns of cards at least 150px wide with 24px between them, as `repeat(auto-fill,
/// minmax(150px, 1fr))`, and how wide each is.
pub fn grid_columns(width: f32) -> (usize, f32) {
    let columns = ((width + 24.) / 174.).floor().max(1.);
    // Rounded down: a card a hair too wide pushes the last column onto the next row.
    (
        columns as usize,
        ((width - 24. * (columns - 1.)) / columns).floor(),
    )
}

impl Render for LibraryPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        let width = content_width(window);
        let (_, card_w) = grid_columns(width);
        let query = self.query.read(cx).text().to_string();

        let head = div()
            .flex()
            .items_baseline()
            .justify_between()
            .gap(px(16.))
            .child(
                div()
                    .text_size(px(34.))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child("Библиотека"),
            )
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .child(more_link(
                        "history",
                        "История",
                        accent,
                        self.open(Route::History, cx),
                    ))
                    .child(more_link(
                        "stats",
                        "Статистика",
                        accent,
                        self.open(Route::Stats, cx),
                    )),
            );

        let tabs = div().flex().flex_wrap().gap(px(8.)).children(TABS.map(|t| {
            chip(t, t == self.status, accent)
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.status = t;
                    if !page.lists.contains_key(t) {
                        page.load(cx);
                    }
                    cx.notify();
                }))
                .child(library_status(t))
        }));

        let options: Vec<SelectOption> = SORTS
            .iter()
            .map(|(_, key, label)| SelectOption {
                value: key.to_string(),
                label: label.to_string(),
                hint: None,
            })
            .collect();
        let current = SORTS
            .iter()
            .find(|(s, ..)| *s == self.sort)
            .map_or("recent", |(_, key, _)| key);
        let page = cx.entity().downgrade();
        let order = select(
            "library-sort",
            &self.sort_menu,
            current,
            &options,
            accent,
            Rc::new(move |key, _, cx| {
                page.update(cx, |page, cx| {
                    if let Some((s, ..)) = SORTS.iter().find(|(_, k, _)| *k == key) {
                        page.sort = *s;
                        cx.notify();
                    }
                })
                .ok();
            }),
            window,
            cx,
        );
        let toolbar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .flex_1()
                    .min_w(px(220.))
                    .max_w(px(420.))
                    .child(self.query.clone()),
            )
            .child(div().ml_auto().child(order));

        let body: AnyElement = match self.lists.get(self.status) {
            None | Some(Load::Pending) => div()
                .flex()
                .flex_wrap()
                .gap_x(px(24.))
                .gap_y(px(32.))
                .children((0..6).map(|_| {
                    div()
                        .w(px(card_w))
                        .h(px(card_w * 1.5))
                        .rounded(theme::R_M)
                        .bg(theme::glass())
                }))
                .into_any_element(),
            Some(Load::Failed(e)) => div()
                .text_color(rgb(theme::DANGER))
                .child(e.clone())
                .into_any_element(),
            Some(Load::Ready(list)) if list.is_empty() => div()
                .text_color(rgb(theme::TEXT_2))
                .max_w(px(560.))
                .child("Здесь пусто. Открой любой тайтл и нажми «В список», или просто начни смотреть: он сам появится в «Смотрю».")
                .into_any_element(),
            Some(Load::Ready(list)) => {
                let variants = query_variants(&query);
                let mut shown: Vec<&LibraryItem> =
                    list.iter().filter(|i| matches(i, &variants)).collect();
                sort(&mut shown, self.sort);
                if shown.is_empty() {
                    div()
                        .text_color(rgb(theme::TEXT_2))
                        .child(format!("В этом списке нет «{query}»."))
                        .into_any_element()
                } else {
                    let (columns, _) = grid_columns(width);
                    let screen = lazy_grid::screen(window);
                    let top = self.grid_top.clone();
                    lazy_grid(&top, shown.len(), columns, row_height(card_w), screen, |i| {
                            let item = shown[i];
                            let a = &item.card.anime;
                            let total = if a.episodes > 0 { a.episodes } else { a.episodes_aired };
                            let text = CardText {
                                note: Some(if total > 0 {
                                    format!("{} из {total}", item.watched)
                                } else {
                                    format!("{} просмотрено", item.watched)
                                }),
                                fraction: (total > 0).then(|| item.watched as f64 / total as f64),
                                badge: None,
                            };
                            let open = self.open(Route::Title(a.id), cx);
                            poster_card_sized(&item.card, "library", i, text, card_w, accent, open, window, cx)
                                .into_any_element()
                        })
                }
            }
        };

        div()
            .flex()
            .flex_col()
            .gap(px(32.))
            .px(px(48.))
            .pt(px(50.))
            .pb(px(72.))
            .child(head)
            .child(tabs)
            .child(toolbar)
            .child(body)
    }
}
