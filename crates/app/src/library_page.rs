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
use crate::catalog::toggle;
use crate::catalog_page::{KINDS, Options, SCORES, STATUSES, filters_toggle, row};
use crate::core::Core;
use crate::data::{Load, fetch};
use crate::episode_grid::chip;
use crate::format::library_status;
use crate::lazy_grid::{self, GridTop, lazy_grid, row_height};
use crate::library::{Filter, SORTS, Sort, choices, matches, query_variants, sort};
use crate::nav::PageEvent;
use crate::nav::Route;
use crate::pinned::{self, Pin};
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
    pin: Pin,
    status: &'static str,
    sort: Sort,
    sort_menu: Menu,
    query: Entity<TextInput>,
    filter: Filter,
    filters_open: bool,
    /// Times the filters were opened: each opening is a new entrance.
    opened: u64,
    lists: HashMap<&'static str, Load<Vec<LibraryItem>>>,
}

impl EventEmitter<Open> for LibraryPage {}
impl EventEmitter<PageEvent> for LibraryPage {}

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
            pin: Pin::default(),
            status: "watching",
            sort: Sort::Recent,
            sort_menu: Menu::default(),
            query,
            filter: Filter::default(),
            filters_open: false,
            opened: 0,
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

impl LibraryPage {
    /// The status tabs, the search and the sort: in the page, or pinned along the top.
    fn controls(
        &mut self,
        gap: f32,
        accent: gpui::Hsla,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
            .child(self.filters_bar(accent, cx))
            .child(div().ml_auto().child(order));
        let filters = self.filters_open.then(|| {
            crate::motion::rise(
                self.filters(accent, cx),
                ("filters", self.opened),
                0,
                200,
                0,
                -10.,
            )
        });
        div()
            .flex()
            .flex_col()
            .gap(px(gap))
            .child(tabs)
            .child(toolbar)
            .children(filters)
            .into_any_element()
    }

    fn filters_bar(&self, accent: gpui::Hsla, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.filter.count();
        let toggle = filters_toggle(self.filters_open, count, accent).on_click(cx.listener(
            |page, _, _, cx| {
                page.filters_open = !page.filters_open;
                page.opened += 1;
                cx.notify();
            },
        ));
        let reset = (count > 0).then(|| {
            div()
                .id("library-reset")
                .cursor_pointer()
                .px(px(14.))
                .py(px(8.))
                .rounded_full()
                .text_size(px(13.))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(theme::TEXT_2))
                .hover(|s| s.bg(theme::glass_strong()).text_color(rgb(theme::TEXT)))
                .on_click(cx.listener(|page, _, _, cx| {
                    page.filter = Filter::default();
                    cx.notify();
                }))
                .child("Сбросить")
        });
        div()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(toggle)
            .children(reset)
    }

    /// Chips that turn the values of a list filter on and off.
    fn toggles(
        &self,
        key: &'static str,
        options: &'static Options,
        field: fn(&mut Filter) -> &mut Vec<String>,
        accent: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let current = field(&mut self.filter.clone()).clone();
        options
            .iter()
            .enumerate()
            .map(|(i, (value, label))| {
                chip((key, i), current.iter().any(|v| v == value), accent)
                    .on_click(cx.listener(move |page, _, _, cx| {
                        toggle(field(&mut page.filter), value);
                        cx.notify();
                    }))
                    .child(*label)
                    .into_any_element()
            })
            .collect()
    }

    fn filters(&self, accent: gpui::Hsla, cx: &mut Context<Self>) -> AnyElement {
        let (decades, genres) = match self.lists.get(self.status) {
            Some(Load::Ready(list)) => choices(list),
            _ => Default::default(),
        };
        let score = self.filter.score;
        let mut scores = vec![
            chip("library-score-any", score.is_none(), accent)
                .on_click(cx.listener(|page, _, _, cx| {
                    page.filter.score = None;
                    cx.notify();
                }))
                .child("Любая")
                .into_any_element(),
        ];
        scores.extend(SCORES.map(|n| {
            chip(("library-score", n as u64), score == Some(n), accent)
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.filter.score = Some(n);
                    cx.notify();
                }))
                .child(format!("от {n}"))
                .into_any_element()
        }));
        let decade_chips = decades
            .into_iter()
            .map(|d| {
                let on = self.filter.decades.contains(&d);
                chip(("library-decade", d as u64), on, accent)
                    .on_click(cx.listener(move |page, _, _, cx| {
                        let all = &mut page.filter.decades;
                        match all.iter().position(|x| *x == d) {
                            Some(i) => {
                                all.remove(i);
                            }
                            None => all.push(d),
                        }
                        cx.notify();
                    }))
                    .child(format!("{d}-е"))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let genre_chips = genres
            .into_iter()
            .enumerate()
            .map(|(i, g)| {
                let on = self.filter.genres.contains(&g);
                let label = g.clone();
                chip(("library-genre", i), on, accent)
                    .on_click(cx.listener(move |page, _, _, cx| {
                        toggle(&mut page.filter.genres, &g);
                        cx.notify();
                    }))
                    .child(label)
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(24.))
            .rounded(theme::R_L)
            .bg(theme::glass())
            .border_1()
            .border_color(theme::line())
            .child(row(
                "Тип",
                self.toggles("library-kind", KINDS, |f| &mut f.kinds, accent, cx),
            ))
            .child(row(
                "Статус",
                self.toggles("library-status", STATUSES, |f| &mut f.statuses, accent, cx),
            ))
            .when(!decade_chips.is_empty(), |d| {
                d.child(row("Годы", decade_chips))
            })
            .child(row("Оценка", scores))
            .when(!genre_chips.is_empty(), |d| {
                d.child(row("Жанры", genre_chips))
            })
            .into_any_element()
    }

    /// What the controls show, for the pinned band to know when to draw them again.
    pub fn controls_key(&self) -> String {
        let list = match self.lists.get(self.status) {
            Some(Load::Ready(list)) => list.len() as i64,
            Some(Load::Failed(_)) => -2,
            _ => -1,
        };
        format!(
            "{}|{:?}|{:?}|{}|{list}",
            self.status, self.sort, self.filter, self.filters_open
        )
    }

    pub fn is_pinned(&self) -> bool {
        self.pin.pinned()
    }

    /// The controls pinned along the top once the page has scrolled past them.
    pub fn pinned(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let accent = Accent::colors(cx).0;
        self.pin
            .pinned()
            .then(|| pinned::band(self.controls(12., accent, window, cx), window))
    }
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
            );

        let controls = (!self.pin.pinned()).then(|| self.controls(32., accent, window, cx));

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
                let mut shown: Vec<&LibraryItem> = list
                    .iter()
                    .filter(|i| matches(i, &variants) && self.filter.passes(i))
                    .collect();
                sort(&mut shown, self.sort);
                if shown.is_empty() {
                    div()
                        .text_color(rgb(theme::TEXT_2))
                        .child(if query.trim().is_empty() {
                            "В этом списке нет тайтлов с такими фильтрами.".to_string()
                        } else {
                            format!("В этом списке нет «{query}».")
                        })
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
            .child(self.pin.slot(controls))
            .child(body)
    }
}
