//! The catalog: ready-made collections, every filter Shikimori has, an order, and
//! the titles in a grid that loads more as its end comes near.

use std::collections::HashSet;
use std::f32::consts::PI;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, Div, EventEmitter, FontWeight, Hsla, IntoElement, Render, Transformation,
    Window, canvas, div, prelude::*, px, radians, rgb,
};
use ninari_core::catalog::browse::{BrowseFilter, Genre, PAGE_SIZE};
use ninari_core::catalog::calendar::civil_from_days;
use ninari_core::feeds::{self, AnimeCard};

use crate::assets::icon;
use crate::button::{Size, Variant, button};
use crate::cards::{CardText, poster_card_sized};
use crate::catalog::{
    Mark, active_filters, cycle_genre, genre_mark, is_named_season, preset_of, presets, season_at,
    season_title, shift_season, toggle, years, years_season,
};
use crate::clock::now;
use crate::core::Core;
use crate::data::{Load, fetch};
use crate::episode_grid::chip;
use crate::lazy_grid::{self, GridTop, lazy_grid, row_height};
use crate::library::swap_layout;
use crate::library_page::Open;
use crate::library_page::{content_width, grid_columns};
use crate::nav::{PageEvent, Route};
use crate::pinned::{self, Pin};
use crate::popover::Menu;
use crate::search_page::{SearchEvent, SearchPage};
use crate::select::{SelectOption, select};
use crate::theme::{self, Accent};

pub type Options = [(&'static str, &'static str)];

pub const KINDS: &Options = &[
    ("tv", "Сериал"),
    ("movie", "Фильм"),
    ("ova", "OVA"),
    ("ona", "ONA"),
    ("special", "Спешл"),
    ("tv_special", "TV-спешл"),
];
pub const STATUSES: &Options = &[
    ("ongoing", "Онгоинг"),
    ("released", "Вышел"),
    ("anons", "Анонс"),
];
const RATINGS: &Options = &[
    ("g", "G"),
    ("pg", "PG"),
    ("pg_13", "PG-13"),
    ("r", "R-17"),
    ("r_plus", "R+"),
];
const DURATIONS: &Options = &[
    ("S", "до 10 мин"),
    ("D", "до 30 мин"),
    ("F", "дольше 30 мин"),
];
pub const SCORES: [u32; 3] = [6, 7, 8];
const GROUPS: &Options = &[
    ("genre", "Жанры"),
    ("theme", "Темы"),
    ("demographic", "Аудитория"),
];
/// Themes are many; the rest open on demand.
const THEMES_SHOWN: usize = 14;
const ORDERS: &Options = &[
    ("popularity", "Сначала популярные"),
    ("ranked", "По рейтингу"),
    ("aired_on", "По дате выхода"),
    ("name", "По названию"),
];
/// What a title's link asks the catalog to show.
#[derive(Clone)]
pub enum Seed {
    Genre(String),
    Studio(i64, String),
}

pub struct CatalogPage {
    core: Arc<Core>,
    /// The search field, recent queries and the search by frame; the query goes into `filter`.
    search: gpui::Entity<SearchPage>,
    /// Nothing was found as typed, so the query in the other keyboard layout was: (typed, used).
    swapped: Option<(String, String)>,
    grid_top: GridTop,
    pin: Pin,
    filter: BrowseFilter,
    studio_name: Option<String>,
    /// The season on now, for the collection and the season chip.
    season: String,
    filters_open: bool,
    all_themes: bool,
    genres: Load<Vec<Genre>>,
    /// A genre a title's link asked for, found by name once the list is in.
    wanted_genre: Option<String>,
    cards: Load<Vec<AnimeCard>>,
    pages: u32,
    more: bool,
    loading: bool,
    more_error: Option<String>,
    /// Bumped by every new filter, so answers for an old one are dropped.
    generation: u64,
    order_menu: Menu,
    from_menu: Menu,
    to_menu: Menu,
}

impl EventEmitter<PageEvent> for CatalogPage {}

impl CatalogPage {
    pub fn new(core: Arc<Core>, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| SearchPage::new(core.clone(), cx));
        cx.subscribe(&search, |page, _, event: &SearchEvent, cx| match event {
            SearchEvent::Query(q) => {
                page.swapped = None;
                let q = Some(q.clone()).filter(|q| !q.is_empty());
                page.patch(cx, |f| f.search = q);
            }
            SearchEvent::Open(route) => cx.emit(PageEvent::Open(*route)),
            SearchEvent::Play(request) => cx.emit(PageEvent::Play(request.clone())),
        })
        .detach();
        cx.subscribe(&search, |_, _, Open(route), cx| {
            cx.emit(PageEvent::Open(*route))
        })
        .detach();
        let page = Self {
            core,
            search,
            swapped: None,
            grid_top: GridTop::default(),
            pin: Pin::default(),
            filter: BrowseFilter::default(),
            studio_name: None,
            season: season_at(now()),
            filters_open: false,
            all_themes: false,
            genres: Load::Pending,
            wanted_genre: None,
            cards: Load::Pending,
            pages: 0,
            more: false,
            loading: false,
            more_error: None,
            generation: 0,
            order_menu: Menu::default(),
            from_menu: Menu::default(),
            to_menu: Menu::default(),
        };
        fetch(
            cx,
            page.core.run(|c| async move { c.catalog.genres().await }),
            |page, genres| {
                page.genres = genres;
                if let Some(name) = page.wanted_genre.take() {
                    page.want_genre(name);
                }
            },
        );
        page
    }

    /// The search field takes the keyboard (the catalog was opened to search).
    pub fn focus_search(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.focus(window, cx));
    }

    /// Puts `text` in the search and searches it at once.
    pub fn set_query(&mut self, text: &str, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.set_query(text, cx));
    }

    /// A title's genre or studio link: the catalog of just that.
    pub fn show(&mut self, seed: Seed, cx: &mut Context<Self>) {
        self.filters_open = true;
        match seed {
            Seed::Studio(id, name) => {
                self.set(
                    BrowseFilter {
                        studio: Some(id),
                        ..Default::default()
                    },
                    cx,
                );
                self.studio_name = Some(name);
            }
            Seed::Genre(name) => {
                if matches!(self.genres, Load::Pending) {
                    self.wanted_genre = Some(name);
                } else {
                    self.want_genre(name);
                }
                cx.notify();
            }
        }
    }

    fn want_genre(&mut self, name: String) {
        let Load::Ready(list) = &self.genres else {
            return;
        };
        if let Some(g) = list.iter().find(|g| g.name.eq_ignore_ascii_case(&name)) {
            self.filter = BrowseFilter {
                genres: vec![g.id],
                ..Default::default()
            };
            self.studio_name = None;
            self.reload();
        }
    }

    fn set(&mut self, filter: BrowseFilter, cx: &mut Context<Self>) {
        if filter == self.filter {
            return;
        }
        if filter.studio.is_none() {
            self.studio_name = None;
        }
        self.filter = filter;
        self.reload();
        cx.notify();
    }

    fn patch(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut BrowseFilter)) {
        let mut next = self.filter.clone();
        change(&mut next);
        self.set(next, cx);
    }

    /// The list starts over; the next frame fetches its first page.
    fn reload(&mut self) {
        self.generation += 1;
        self.cards = Load::Pending;
        self.pages = 0;
        self.more = false;
        self.loading = false;
        self.more_error = None;
    }

    /// The next page, when the end of the list comes near.
    fn next_page(&mut self, cx: &mut Context<Self>) {
        if self.more && !self.loading && self.more_error.is_none() {
            self.load(cx);
            cx.notify();
        }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let (filter, page, generation) = (self.filter.clone(), self.pages + 1, self.generation);
        self.loading = true;
        fetch(
            cx,
            self.core.run(move |c| async move {
                let list = c.catalog.browse(&filter, page).await?;
                // Nothing as typed: perhaps it was typed with the other keyboard layout on.
                let (list, used) = match filter.search.as_deref().map(|q| (q, swap_layout(q))) {
                    Some((q, swapped)) if page == 1 && list.is_empty() && swapped != q => {
                        let other = BrowseFilter {
                            search: Some(swapped.clone()),
                            ..filter.clone()
                        };
                        (c.catalog.browse(&other, 1).await?, Some(swapped))
                    }
                    _ => (list, None),
                };
                let cards = list
                    .into_iter()
                    .map(|a| feeds::card(&c.posters, a))
                    .collect::<Vec<_>>();
                Ok((cards, used))
            }),
            move |p, got| {
                if p.generation != generation {
                    return;
                }
                p.loading = false;
                match got {
                    Load::Ready((list, used)) => {
                        if let Some(used) = used.filter(|_| !list.is_empty()) {
                            // The next pages go on with what found something.
                            let typed = p.filter.search.replace(used.clone()).unwrap_or_default();
                            p.swapped = Some((typed, used));
                        }
                        p.pages = page;
                        p.more = list.len() >= PAGE_SIZE as usize;
                        match &mut p.cards {
                            // Random order may repeat a title on the next page.
                            Load::Ready(cards) => {
                                let seen: HashSet<i64> = cards.iter().map(|c| c.anime.id).collect();
                                cards.extend(
                                    list.into_iter().filter(|c| !seen.contains(&c.anime.id)),
                                );
                            }
                            cards => *cards = Load::Ready(list),
                        }
                    }
                    Load::Failed(e) if page == 1 => p.cards = Load::Failed(e),
                    Load::Failed(e) => p.more_error = Some(e),
                    Load::Pending => {}
                }
            },
        );
    }

    /// Chips that turn the values of a list filter on and off.
    fn toggles(
        &self,
        key: &'static str,
        options: &'static Options,
        field: fn(&mut BrowseFilter) -> &mut Vec<String>,
        accent: Hsla,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let current = field(&mut self.filter.clone()).clone();
        options
            .iter()
            .enumerate()
            .map(|(i, (value, label))| {
                chip((key, i), current.iter().any(|v| v == value), accent)
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.patch(cx, |f| toggle(field(f), value))
                    }))
                    .child(*label)
                    .into_any_element()
            })
            .collect()
    }

    fn when(&self, accent: Hsla, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let year = civil_from_days(now().div_euclid(86400)).0;
        let season = self.filter.season.clone();
        let named = season.as_deref().is_some_and(is_named_season);
        let range = season.as_deref().and_then(years);
        let mut chips = vec![
            chip("when-any", season.is_none(), accent)
                .on_click(cx.listener(|page, _, _, cx| page.patch(cx, |f| f.season = None)))
                .child("Любое время")
                .into_any_element(),
            chip("when-season", named, accent)
                .on_click(cx.listener(move |page, _, _, cx| {
                    if !named {
                        let now = page.season.clone();
                        page.patch(cx, |f| f.season = Some(now));
                    }
                }))
                .child("Сезон")
                .into_any_element(),
            chip("when-years", range.is_some(), accent)
                .on_click(cx.listener(move |page, _, _, cx| {
                    if range.is_none() {
                        page.patch(cx, |f| f.season = Some(years_season(year - 5, year)));
                    }
                }))
                .child("Годы")
                .into_any_element(),
        ];
        if let (true, Some(s)) = (named, season.as_deref()) {
            // The next one is the same chevron turned round.
            let step = |id: &'static str, turn: f32, delta: i64, cx: &mut Context<Self>| {
                div()
                    .id(id)
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(32.))
                    .rounded_full()
                    .bg(theme::glass())
                    .border_1()
                    .border_color(theme::line())
                    .hover(|s| s.bg(theme::glass_strong()))
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.patch(cx, |f| {
                            f.season = f.season.as_deref().map(|s| shift_season(s, delta))
                        })
                    }))
                    .child(
                        icon("back", 18.)
                            .text_color(rgb(theme::TEXT_2))
                            .with_transformation(Transformation::rotate(radians(turn))),
                    )
            };
            chips.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .ml(px(8.))
                    .child(step("season-prev", 0., -1, cx))
                    .child(
                        div()
                            .min_w(px(110.))
                            .text_center()
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child(season_title(s)),
                    )
                    .child(step("season-next", PI, 1, cx))
                    .into_any_element(),
            );
        }
        if let Some((from, to)) = range {
            let options: Vec<SelectOption> = (1960..=year + 1)
                .rev()
                .map(|y| SelectOption {
                    value: y.to_string(),
                    label: y.to_string(),
                    hint: None,
                })
                .collect();
            let page = cx.entity().downgrade();
            let pick = move |first: bool| -> crate::select::OnPick {
                let page = page.clone();
                Rc::new(move |value, _, cx| {
                    let Ok(y) = value.parse::<i64>() else {
                        return;
                    };
                    let (a, b) = if first { (y, to) } else { (from, y) };
                    page.update(cx, |page, cx| {
                        page.patch(cx, |f| f.season = Some(years_season(a, b)))
                    })
                    .ok();
                })
            };
            let (pick_from, pick_to) = (pick(true), pick(false));
            chips.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .ml(px(8.))
                    .child(select(
                        "catalog-from",
                        &self.from_menu,
                        &from.to_string(),
                        &options,
                        accent,
                        pick_from,
                        window,
                        cx,
                    ))
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_3))
                            .text_size(px(13.))
                            .child("по"),
                    )
                    .child(select(
                        "catalog-to",
                        &self.to_menu,
                        &to.to_string(),
                        &options,
                        accent,
                        pick_to,
                        window,
                        cx,
                    ))
                    .into_any_element(),
            );
        }
        row("Когда", chips)
    }

    fn genre_rows(&self, accent: Hsla, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let list = match &self.genres {
            Load::Ready(list) => list,
            Load::Pending => return vec![hint("Загружаем жанры").into_any_element()],
            Load::Failed(e) => {
                return vec![
                    div()
                        .text_color(rgb(theme::DANGER))
                        .text_size(px(13.))
                        .child(e.clone())
                        .into_any_element(),
                ];
            }
        };
        let danger: Hsla = rgb(theme::DANGER).into();
        let mut rows: Vec<AnyElement> = GROUPS
            .iter()
            .filter_map(|(kind, label)| {
                let group: Vec<&Genre> = list.iter().filter(|g| g.kind == *kind).collect();
                if group.is_empty() {
                    return None;
                }
                let cut = *kind == "theme" && !self.all_themes && group.len() > THEMES_SHOWN;
                // A marked theme stays in sight even when the list is cut.
                let shown: Vec<&Genre> = group
                    .iter()
                    .enumerate()
                    .filter(|(i, g)| {
                        !cut || *i < THEMES_SHOWN || genre_mark(&self.filter, g.id).is_some()
                    })
                    .map(|(_, g)| *g)
                    .collect();
                let mut chips: Vec<AnyElement> =
                    shown
                        .iter()
                        .map(|g| {
                            let id = g.id;
                            let el = match genre_mark(&self.filter, id) {
                                // Struck through in red: "not this".
                                Some(Mark::Exclude) => chip(("genre", id as u64), true, danger)
                                    .bg(danger.opacity(0.14))
                                    .border_color(danger.opacity(0.45))
                                    .text_color(rgb(theme::TEXT_2))
                                    .line_through(),
                                mark => chip(("genre", id as u64), mark.is_some(), accent),
                            };
                            el.on_click(cx.listener(move |page, _, _, cx| {
                                page.patch(cx, |f| cycle_genre(f, id))
                            }))
                            .child(g.name.clone())
                            .into_any_element()
                        })
                        .collect();
                if cut {
                    chips.push(
                        div()
                            .id("all-themes")
                            .cursor_pointer()
                            .px(px(12.))
                            .py(px(7.))
                            .text_color(accent)
                            .text_size(px(13.))
                            .font_weight(FontWeight::BOLD)
                            .hover(|s| s.underline())
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.all_themes = true;
                                cx.notify();
                            }))
                            .child(format!("Ещё {}", group.len() - shown.len()))
                            .into_any_element(),
                    );
                }
                Some(row(label, chips).into_any_element())
            })
            .collect();
        rows.push(
            hint("Второе нажатие на жанр исключает его из выдачи, третье снимает.")
                .pl(px(122.))
                .into_any_element(),
        );
        rows
    }

    fn filters(&self, accent: Hsla, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let score = self.filter.score;
        let mut scores = vec![
            chip("score-any", score.is_none(), accent)
                .on_click(cx.listener(|page, _, _, cx| page.patch(cx, |f| f.score = None)))
                .child("Любая")
                .into_any_element(),
        ];
        scores.extend(SCORES.map(|n| {
            chip(("score", n as u64), score == Some(n), accent)
                .on_click(cx.listener(move |page, _, _, cx| page.patch(cx, |f| f.score = Some(n))))
                .child(format!("от {n}"))
                .into_any_element()
        }));
        let studio = self.filter.studio.map(|id| {
            let name = self.studio_name.clone().unwrap_or_else(|| format!("#{id}"));
            row(
                "Студия",
                vec![
                    chip("studio", true, accent)
                        .on_click(cx.listener(|page, _, _, cx| page.patch(cx, |f| f.studio = None)))
                        .child(name)
                        .child(icon("close", 16.).text_color(rgb(theme::TEXT)))
                        .into_any_element(),
                ],
            )
        });
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
                self.toggles("kind", KINDS, |f| &mut f.kinds, accent, cx),
            ))
            .child(row(
                "Статус",
                self.toggles("status", STATUSES, |f| &mut f.statuses, accent, cx),
            ))
            .child(self.when(accent, window, cx))
            .child(row("Оценка", scores))
            .child(row(
                "Возраст",
                self.toggles("rating", RATINGS, |f| &mut f.ratings, accent, cx),
            ))
            .child(row(
                "Длина серии",
                self.toggles("duration", DURATIONS, |f| &mut f.durations, accent, cx),
            ))
            .children(studio)
            .children(self.genre_rows(accent, cx))
    }

    fn bar(&self, accent: Hsla, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let count = active_filters(&self.filter);
        let open = self.filters_open;
        let toggle = filters_toggle(open, count, accent).on_click(cx.listener(|page, _, _, cx| {
            page.filters_open = !page.filters_open;
            cx.notify();
        }));
        let reset = (count > 0).then(|| {
            div()
                .id("reset")
                .cursor_pointer()
                .px(px(14.))
                .py(px(8.))
                .rounded_full()
                .text_size(px(13.))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(theme::TEXT_2))
                .hover(|s| s.bg(theme::glass_strong()).text_color(rgb(theme::TEXT)))
                .on_click(cx.listener(|page, _, _, cx| {
                    let order = page.filter.order.clone();
                    page.set(
                        BrowseFilter {
                            order,
                            ..Default::default()
                        },
                        cx,
                    );
                }))
                .child("Сбросить")
        });
        // A search comes best match first unless another order is picked.
        let searching = self.filter.search.is_some();
        let unset = if searching { "relevance" } else { "popularity" };
        let options: Vec<SelectOption> = searching
            .then_some(&("relevance", "Сначала совпадения"))
            .into_iter()
            .chain(ORDERS.iter())
            .map(|(value, label)| SelectOption {
                value: value.to_string(),
                label: label.to_string(),
                hint: None,
            })
            .collect();
        let page = cx.entity().downgrade();
        let order = select(
            "catalog-order",
            &self.order_menu,
            self.filter.order.as_deref().unwrap_or(unset),
            &options,
            accent,
            Rc::new(move |value, _, cx| {
                page.update(cx, |page, cx| {
                    let unset = if page.filter.search.is_some() {
                        "relevance"
                    } else {
                        "popularity"
                    };
                    page.patch(cx, |f| f.order = (value != unset).then_some(value))
                })
                .ok();
            }),
            window,
            cx,
        );
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(toggle)
            .children(reset)
            .child(div().ml_auto().child(order))
    }

    /// The presets, the filter bar and the open filters: in the page, or pinned along the top.
    fn controls(
        &mut self,
        gap: f32,
        accent: Hsla,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let preset = preset_of(&self.filter, &self.season);
        let presets =
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.))
                .children(presets(&self.season).into_iter().map(|p| {
                    let filter = p.filter;
                    chip(p.key, preset == Some(p.key), accent)
                        .on_click(cx.listener(move |page, _, _, cx| page.set(filter.clone(), cx)))
                        .child(p.label)
                }));
        let bar = self.bar(accent, window, cx);
        let filters = self.filters_open.then(|| self.filters(accent, window, cx));
        div()
            .flex()
            .flex_col()
            .gap(px(gap))
            .child(self.search.clone())
            .child(presets)
            .child(bar)
            .children(filters)
            .into_any_element()
    }

    /// The controls pinned along the top once the page has scrolled past them.
    pub fn pinned(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let accent = Accent::colors(cx).0;
        self.pin
            .pinned()
            .then(|| pinned::band(self.controls(12., accent, window, cx), window))
    }

    fn grid(&self, accent: Hsla, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (_, card_w) = grid_columns(content_width(window));
        let grid = || div().flex().flex_wrap().gap_x(px(24.)).gap_y(px(32.));
        match &self.cards {
            Load::Pending => grid()
                .children((0..12).map(|_| {
                    div()
                        .w(px(card_w))
                        .h(px(card_w * 1.5))
                        .rounded(theme::R_M)
                        .bg(theme::glass())
                }))
                .into_any_element(),
            Load::Failed(e) => div()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(12.))
                .text_color(rgb(theme::DANGER))
                .child(e.clone())
                .child(
                    button(
                        "retry",
                        "Повторить",
                        Some("retry"),
                        Variant::Glass,
                        Size::M,
                        accent,
                    )
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.reload();
                        cx.notify();
                    })),
                )
                .into_any_element(),
            Load::Ready(list) if list.is_empty() => div()
                .text_color(rgb(theme::TEXT_2))
                .child(match &self.filter.search {
                    Some(q) => format!(
                        "По запросу «{q}» ничего не нашлось. Попробуй другое написание, \
                         оригинальное название или убери часть фильтров."
                    ),
                    None => {
                        "Ничего не нашлось. Убери часть фильтров или исключённых жанров.".into()
                    }
                })
                .into_any_element(),
            Load::Ready(list) => {
                let (columns, _) = grid_columns(content_width(window));
                let screen = lazy_grid::screen(window);
                let top = self.grid_top.clone();
                let cards = lazy_grid(&top, list.len(), columns, row_height(card_w), screen, |i| {
                    let card = &list[i];
                    let id = card.anime.id;
                    let open = Box::new(cx.listener(move |page, _, _, cx| {
                        // Opening a result is what makes a query worth remembering.
                        if page.filter.search.is_some() {
                            page.search.update(cx, |s, cx| s.remember_current(cx));
                        }
                        cx.emit(PageEvent::Open(Route::Title(id)))
                    }));
                    let text = CardText {
                        note: None,
                        fraction: None,
                        badge: None,
                    };
                    poster_card_sized(card, "catalog", i, text, card_w, accent, open, window, cx)
                        .into_any_element()
                });
                // Stands at the end of the list and asks for more once it is near the screen.
                let sentinel =
                    (self.more && !self.loading && self.more_error.is_none()).then(|| {
                        let page = cx.entity().downgrade();
                        canvas(
                            move |bounds, window, cx| {
                                if bounds.top() < window.viewport_size().height + px(800.) {
                                    let page = page.clone();
                                    cx.defer(move |cx| {
                                        page.update(cx, |page, cx| page.next_page(cx)).ok();
                                    });
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .w_full()
                        .h(px(1.))
                    });
                let tail = if self.loading {
                    Some(hint("Загружаем ещё").text_center().into_any_element())
                } else {
                    self.more_error.clone().map(|e| {
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(px(12.))
                            .text_size(px(13.))
                            .text_color(rgb(theme::DANGER))
                            .child(e)
                            .child(
                                button(
                                    "more-retry",
                                    "Повторить",
                                    Some("retry"),
                                    Variant::Glass,
                                    Size::M,
                                    accent,
                                )
                                .on_click(cx.listener(
                                    |page, _, _, cx| {
                                        page.more_error = None;
                                        page.next_page(cx);
                                    },
                                )),
                            )
                            .into_any_element()
                    })
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(32.))
                    .child(cards)
                    .children(sentinel)
                    .children(tail)
                    .into_any_element()
            }
        }
    }
}

/// The button that opens the filters, with how many are on.
pub fn filters_toggle(open: bool, count: usize, accent: Hsla) -> gpui::Stateful<Div> {
    div()
        .id("filters")
        .cursor_pointer()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(14.))
        .py(px(8.))
        .rounded_full()
        .border_1()
        .border_color(theme::line())
        .text_size(px(13.))
        .font_weight(FontWeight::BOLD)
        .map(|d| {
            if open {
                d.bg(theme::glass_strong()).text_color(rgb(theme::TEXT))
            } else {
                d.bg(theme::glass())
                    .text_color(rgb(theme::TEXT_2))
                    .hover(|s| s.bg(theme::glass_strong()).text_color(rgb(theme::TEXT)))
            }
        })
        .child(icon("filter", 18.).text_color(rgb(if open { theme::TEXT } else { theme::TEXT_2 })))
        .child("Фильтры")
        .when(count > 0, |d| {
            d.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(px(20.))
                    .h(px(20.))
                    .px(px(6.))
                    .rounded_full()
                    .bg(accent)
                    .text_color(rgb(theme::ACCENT_CONTRAST))
                    .text_size(px(12.))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child(count.to_string()),
            )
        })
}

/// A label in one column, its chips wrapping beside it.
pub fn row(label: &'static str, chips: Vec<AnyElement>) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(12.))
        .child(
            div()
                .flex_none()
                .w(px(110.))
                .pt(px(7.))
                .text_size(px(13.))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(theme::TEXT_3))
                .child(label),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_w(px(0.))
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .children(chips),
        )
}

fn hint(text: &'static str) -> Div {
    div()
        .text_size(px(12.))
        .text_color(rgb(theme::TEXT_3))
        .child(text)
}

impl Render for CatalogPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        if matches!(self.cards, Load::Pending) && !self.loading {
            self.load(cx);
        }
        let head = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .child(
                div()
                    .text_size(px(34.))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child("Каталог"),
            )
            .child(
                button(
                    "random",
                    "Случайный тайтл",
                    Some("shuffle"),
                    Variant::Glass,
                    Size::M,
                    accent,
                )
                // The title opens at once, as from the rail.
                .on_click(cx.listener(|_, _, _, cx| cx.emit(PageEvent::Random))),
            );

        let controls = (!self.pin.pinned()).then(|| self.controls(32., accent, window, cx));
        let grid = self.grid(accent, window, cx);
        div()
            .flex()
            .flex_col()
            .gap(px(32.))
            .px(px(48.))
            .pt(px(50.))
            .pb(px(72.))
            .child(head)
            .child(div().mt(px(-12.)).child(self.pin.slot(controls)))
            .children(self.search.update(cx, |s, cx| s.frame_results(cx)))
            .children(self.swapped.clone().map(|(typed, used)| {
                div()
                    .text_size(px(13.))
                    .text_color(rgb(theme::TEXT_3))
                    .child(format!(
                        "По «{typed}» ничего нет, показаны результаты для «{used}»."
                    ))
                    .into_any_element()
            }))
            .child(grid)
    }
}
