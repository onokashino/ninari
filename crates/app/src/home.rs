//! The home screen: the title to continue, what is being watched, today's episodes, new seasons,
//! the season's popular titles, recommendations and plans. Each list loads on its own and shows
//! when it is in.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    AnyElement, BoxShadow, Context, Div, EventEmitter, FontWeight, Hsla, IntoElement, ObjectFit,
    Render, Window, div, img, linear_color_stop, linear_gradient, point, prelude::*, px, rgb, rgba,
};
use ninari_core::catalog::calendar::unix_of;
use ninari_core::feeds::{self, AnimeCard, ContinueItem, LibraryItem, NewSeasonCard, ScheduleItem};

use crate::OnClick;
use crate::button::{Size, Variant, button};
use crate::cards::{CardText, poster_card, today_card};
use crate::clock::{is_followed, now, today_picks, utc_offset};
use crate::core::Core;
use crate::data::{Load, fetch, title};
use crate::format::{plural, remaining};
use crate::nav::Route;
use crate::player::Request;
use crate::posters::poster;
use crate::scroll_row::Row;
use crate::theme::{self, Accent};

pub enum HomeEvent {
    Open(Route),
    Play(Request),
}

pub struct Feed {
    /// The titles to continue; the first gets the big card.
    pub cont: Vec<ContinueItem>,
    /// The wide AniList picture of the first one.
    pub banner: Option<PathBuf>,
    pub has_extensions: bool,
    pub library_size: usize,
}

#[derive(Default)]
struct Rows {
    watching: Row,
    today: Row,
    sequels: Row,
    season: Row,
    for_you: Row,
    planned: Row,
}

pub struct Home {
    core: Arc<Core>,
    /// Plays the first title as soon as the feed is in (`bun play`).
    autoplay: bool,
    feed: Load<Feed>,
    schedule: Load<Vec<ScheduleItem>>,
    sequels: Load<Vec<NewSeasonCard>>,
    season: Load<Vec<AnimeCard>>,
    for_you: Load<Vec<AnimeCard>>,
    planned: Load<Vec<LibraryItem>>,
    rows: Rows,
}

impl EventEmitter<HomeEvent> for Home {}

impl Home {
    pub fn new(core: Arc<Core>, autoplay: bool, cx: &mut Context<Self>) -> Self {
        let mut home = Self {
            core,
            autoplay,
            feed: Load::Pending,
            schedule: Load::Pending,
            sequels: Load::Pending,
            season: Load::Pending,
            for_you: Load::Pending,
            planned: Load::Pending,
            rows: Rows::default(),
        };
        home.reload(true, cx);
        home
    }

    /// Fetches what watching changes; `all` also the lists that change slowly.
    pub fn reload(&mut self, all: bool, cx: &mut Context<Self>) {
        let core = &self.core;
        fetch(
            cx,
            core.run(|c| async move {
                let cont = feeds::continue_items(&c.library, &c.catalog, &c.resolver, &c.posters)?;
                let banner = match cont.first() {
                    Some(first) => c.posters.banner(&first.card.anime).await.ok().flatten(),
                    None => None,
                };
                Ok(Feed {
                    cont,
                    banner,
                    has_extensions: !c.host.list().is_empty(),
                    library_size: c.library.list(None)?.len(),
                })
            }),
            |home, feed| home.feed = feed,
        );
        fetch(
            cx,
            core.run(|c| async move {
                feeds::library_items(&c.library, &c.catalog, &c.posters, Some("planned")).await
            }),
            |home, v| home.planned = v,
        );
        if !all {
            return;
        }
        fetch(
            cx,
            core.run(|c| async move {
                feeds::schedule(&c.schedule, &c.catalog, &c.library, &c.posters).await
            }),
            |home, v| home.schedule = v,
        );
        fetch(
            cx,
            core.run(
                |c| async move { feeds::new_seasons(&c.catalog, &c.library, &c.posters).await },
            ),
            |home, v| home.sequels = v,
        );
        fetch(
            cx,
            core.run(|c| async move {
                let list = c.catalog.season_top(20).await?;
                Ok(list
                    .into_iter()
                    .map(|a| feeds::card(&c.posters, a))
                    .collect())
            }),
            |home, v| home.season = v,
        );
        fetch(
            cx,
            core.run(|c| async move {
                feeds::recommendations(&c.library, &c.catalog, &c.posters).await
            }),
            |home, v| home.for_you = v,
        );
    }

    /// The title whose colours the screen takes.
    pub fn hero_card(&self) -> Option<&AnimeCard> {
        match &self.feed {
            Load::Ready(feed) => feed.cont.first().map(|c| &c.card),
            _ => None,
        }
    }

    /// What playing `c` asks the player for.
    fn request(c: &ContinueItem, cx: &mut Context<Self>) -> Option<Request> {
        Some(Request {
            id: c.card.anime.id,
            episode: c.episode,
            dub_key: Some(c.dub_key.clone()?),
            start: None,
            title: title(&c.card.anime).into(),
            palette: poster(&c.card, cx).map(|(_, p)| p).unwrap_or_default(),
        })
    }

    fn open(&self, route: Route, cx: &mut Context<Self>) -> OnClick {
        Box::new(cx.listener(move |_, _, _, cx| cx.emit(HomeEvent::Open(route))))
    }
}

/// A heading with an optional quiet link on the right, and the list under it.
fn section(heading: &str, more: Option<(&'static str, OnClick)>, accent: Hsla) -> Div {
    div().flex().flex_col().gap(px(16.)).child(
        div()
            .flex()
            .items_baseline()
            .justify_between()
            .gap(px(16.))
            .child(
                div()
                    .text_size(px(18.))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child(heading.to_string()),
            )
            .children(more.map(|(label, on_click)| {
                div()
                    .id(label)
                    .cursor_pointer()
                    .on_click(on_click)
                    .text_size(px(13.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(theme::TEXT_3))
                    .hover(move |s| s.text_color(accent))
                    .child(label)
            })),
    )
}

/// Poster rows keep room around the cards for a hovered card's lift and glow.
const CARD_ROOM: (f32, f32, f32) = (14., 44., 34.);

impl Render for Home {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.autoplay
            && let Load::Ready(feed) = &self.feed
        {
            self.autoplay = false;
            if let Some(request) = feed.cont.first().and_then(|c| Self::request(c, cx)) {
                cx.emit(HomeEvent::Play(request));
            }
        }
        let (accent, glow) = Accent::colors(cx);
        let page = div()
            .flex()
            .flex_col()
            .gap(px(32.))
            .max_w(px(1400.))
            .px(px(48.))
            .pt(px(50.))
            .pb(px(72.));
        let feed = match &self.feed {
            Load::Pending => {
                return page.child(div().h(px(320.)).rounded(theme::R_L).bg(theme::glass()));
            }
            Load::Failed(e) => {
                return page.child(div().text_color(rgb(theme::DANGER)).child(e.clone()));
            }
            Load::Ready(feed) => feed,
        };

        let mut sections: Vec<AnyElement> = Vec::new();
        match feed.cont.first() {
            Some(first) => {
                let cover = poster(&first.card, cx).map(|(p, _)| p);
                let play = Self::request(first, cx).map(|request| -> OnClick {
                    Box::new(
                        cx.listener(move |_, _, _, cx| cx.emit(HomeEvent::Play(request.clone()))),
                    )
                });
                let on_title = self.open(Route::Title(first.card.anime.id), cx);
                sections.push(
                    hero(
                        first,
                        feed.banner.clone(),
                        cover,
                        (accent, glow),
                        on_title,
                        play,
                    )
                    .into_any_element(),
                );
            }
            None => sections.push(self.empty(feed, accent, cx).into_any_element()),
        }

        let rest = &feed.cont[feed.cont.len().min(1)..];
        if !rest.is_empty() {
            let cards: Vec<AnyElement> = rest
                .iter()
                .enumerate()
                .map(|(i, it)| {
                    let note = format!(
                        "Серия {}{}",
                        it.episode,
                        it.dub_name
                            .as_ref()
                            .map(|d| format!(", {d}"))
                            .unwrap_or_default()
                    );
                    let text = CardText {
                        note: Some(note),
                        fraction: (it.duration > 0.).then(|| it.position / it.duration),
                        badge: (it.fresh > 0).then(|| format!("+{}", it.fresh)),
                    };
                    let open = self.open(Route::Title(it.card.anime.id), cx);
                    poster_card(&it.card, "watching", i, text, accent, open, window, cx)
                        .into_any_element()
                })
                .collect();
            let row = self
                .rows
                .watching
                .render("watching", px(24.), CARD_ROOM, cards, window, cx);
            sections.push(
                section("Смотрю сейчас", None, accent)
                    .child(row)
                    .into_any_element(),
            );
        }

        if let Load::Ready(schedule) = &self.schedule {
            let now = now();
            let picks = today_picks(
                schedule,
                |it| unix_of(&it.at),
                |it| is_followed(it.status.as_deref()),
                now,
                utc_offset,
                12,
            );
            if !picks.is_empty() {
                let cards: Vec<AnyElement> = picks
                    .iter()
                    .enumerate()
                    .map(|(i, it)| {
                        let at = unix_of(&it.at).unwrap_or(now);
                        let open = self.open(Route::Title(it.card.anime.id), cx);
                        today_card(it, at, now, i, accent, open, window, cx).into_any_element()
                    })
                    .collect();
                let row =
                    self.rows
                        .today
                        .render("today", px(12.), (6., 20., 14.), cards, window, cx);
                let more = ("Всё расписание", self.open(Route::Schedule, cx));
                sections.push(
                    section("Сегодня выходит", Some(more), accent)
                        .child(row)
                        .into_any_element(),
                );
            }
        }

        if let Load::Ready(list) = &self.sequels
            && !list.is_empty()
        {
            let cards: Vec<AnyElement> = list
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    let text = CardText {
                        note: Some(format!("после «{}»", n.after)),
                        fraction: None,
                        badge: None,
                    };
                    let open = self.open(Route::Title(n.card.anime.id), cx);
                    poster_card(&n.card, "sequels", i, text, accent, open, window, cx)
                        .into_any_element()
                })
                .collect();
            let row = self
                .rows
                .sequels
                .render("sequels", px(24.), CARD_ROOM, cards, window, cx);
            sections.push(
                section("Продолжения досмотренного", None, accent)
                    .child(row)
                    .into_any_element(),
            );
        }

        for (heading, list, name, more) in [
            (
                "Популярное этого сезона",
                &self.season,
                "season",
                Some(("Весь сезон", Route::Catalog)),
            ),
            ("Вам может понравиться", &self.for_you, "for_you", None),
        ] {
            let Load::Ready(list) = list else { continue };
            if list.is_empty() {
                continue;
            }
            let cards: Vec<AnyElement> = list
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let text = CardText {
                        note: None,
                        fraction: None,
                        badge: None,
                    };
                    let open = self.open(Route::Title(c.anime.id), cx);
                    poster_card(c, name, i, text, accent, open, window, cx).into_any_element()
                })
                .collect();
            let row = if name == "season" {
                &self.rows.season
            } else {
                &self.rows.for_you
            };
            let row = row.render(name, px(24.), CARD_ROOM, cards, window, cx);
            let more = more.map(|(label, route)| (label, self.open(route, cx)));
            sections.push(section(heading, more, accent).child(row).into_any_element());
        }

        if let Load::Ready(list) = &self.planned
            && !list.is_empty()
        {
            let cards: Vec<AnyElement> = list
                .iter()
                .enumerate()
                .map(|(i, it)| {
                    let text = CardText {
                        note: None,
                        fraction: None,
                        badge: None,
                    };
                    let open = self.open(Route::Title(it.card.anime.id), cx);
                    poster_card(&it.card, "planned", i, text, accent, open, window, cx)
                        .into_any_element()
                })
                .collect();
            let row = self
                .rows
                .planned
                .render("planned", px(24.), CARD_ROOM, cards, window, cx);
            let more = ("Вся библиотека", self.open(Route::Library, cx));
            sections.push(
                section("В планах", Some(more), accent)
                    .child(row)
                    .into_any_element(),
            );
        }

        page.children(sections)
    }
}

impl Home {
    /// Nothing to continue: what to do first.
    fn empty(&self, feed: &Feed, accent: Hsla, cx: &mut Context<Self>) -> impl IntoElement {
        let (heading, lead) = (
            if feed.library_size > 0 {
                "Всё досмотрено"
            } else {
                "Что будем смотреть?"
            },
            if feed.has_extensions {
                "Найди тайтл, выбери озвучку, и он появится здесь с кнопкой «Продолжить»."
            } else {
                "Сначала подключи репозиторий расширений: без них приложению неоткуда брать серии."
            },
        );
        let search_variant = if feed.has_extensions {
            Variant::Primary
        } else {
            Variant::Glass
        };
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(16.))
            .py(px(48.))
            .max_w(px(560.))
            .child(
                div()
                    .text_size(px(34.))
                    .line_height(px(39.))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child(heading),
            )
            .child(div().text_color(rgb(theme::TEXT_2)).child(lead))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .when(!feed.has_extensions, |d| {
                        d.child(
                            button(
                                "plugins",
                                "Подключить расширения",
                                Some("puzzle"),
                                Variant::Primary,
                                Size::L,
                                accent,
                            )
                            .on_click(self.open(Route::Settings, cx)),
                        )
                    })
                    .child(
                        button(
                            "find",
                            "Найти аниме",
                            Some("search"),
                            search_variant,
                            Size::L,
                            accent,
                        )
                        .on_click(self.open(Route::Search, cx)),
                    ),
            )
    }
}

/// The card to continue, its edge lit by the poster's colour towards the poster.
fn hero(
    c: &ContinueItem,
    banner: Option<PathBuf>,
    cover: Option<PathBuf>,
    (accent, glow): (Hsla, Hsla),
    on_title: OnClick,
    on_play: Option<OnClick>,
) -> impl IntoElement {
    let left = if c.position > 0. {
        remaining(c.position, c.duration)
    } else {
        "с начала".into()
    };
    let where_line = [
        Some(format!("Серия {}", c.episode)),
        c.dub_name.clone(),
        Some(left).filter(|s| !s.is_empty()),
    ];
    let fresh = (c.fresh > 0).then(|| {
        let text = if c.fresh == 1 {
            "вышла новая серия".to_string()
        } else {
            let word = plural(c.fresh, "новая серия", "новые серии", "новых серий");
            format!("вышло {} {word}", c.fresh)
        };
        div()
            .px(px(10.))
            .py(px(1.))
            .rounded_full()
            .bg(accent)
            .text_color(rgb(theme::ACCENT_CONTRAST))
            .font_weight(FontWeight::EXTRA_BOLD)
            .child(text)
    });
    let layer = || {
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .rounded(theme::R_L - px(1.))
    };
    div()
        .rounded(theme::R_L)
        .p(px(1.))
        .bg(linear_gradient(
            105.,
            linear_color_stop(rgba(0xffffff12), 0.),
            linear_color_stop(accent.opacity(0.9), 1.),
        ))
        .child(
            div()
                .relative()
                .h(px(340.))
                .rounded(theme::R_L - px(1.))
                .overflow_hidden()
                .bg(rgb(theme::SURFACE))
                .flex()
                .items_end()
                .justify_between()
                .children(banner.or(cover.clone()).map(|p| {
                    img(p)
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .rounded(theme::R_L - px(1.))
                        .object_fit(ObjectFit::Cover)
                }))
                .child(layer().bg(linear_gradient(
                    90.,
                    linear_color_stop(rgba(0x101117f0), 0.),
                    linear_color_stop(rgba(0x10111726), 1.),
                )))
                .child(layer().bg(linear_gradient(
                    0.,
                    linear_color_stop(glow.opacity(0.55), 0.),
                    linear_color_stop(glow.opacity(0.), 0.6),
                )))
                .child(
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .gap(px(12.))
                        .px(px(48.))
                        .py(px(32.))
                        .max_w(px(640.))
                        .child(
                            div()
                                .id("hero-title")
                                .cursor_pointer()
                                .on_click(on_title)
                                .text_size(px(46.))
                                .line_height(px(48.))
                                .font_weight(FontWeight::EXTRA_BOLD)
                                .child(title(&c.card.anime)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(16.))
                                .text_color(rgb(theme::TEXT_2))
                                .font_weight(FontWeight::SEMIBOLD)
                                .children(where_line.into_iter().flatten())
                                .children(fresh),
                        )
                        .children(on_play.map(|on_play| {
                            div().mt(px(8.)).flex().child(
                                button(
                                    "continue",
                                    "Продолжить",
                                    Some("play"),
                                    Variant::Primary,
                                    Size::L,
                                    accent,
                                )
                                .on_click(on_play),
                            )
                        })),
                )
                .children(cover.map(|p| {
                    img(p)
                        .relative()
                        .w(px(150.))
                        .h(px(225.))
                        .mr(px(48.))
                        .mb(px(32.))
                        .rounded(theme::R_M)
                        .object_fit(ObjectFit::Cover)
                        .shadow(vec![BoxShadow {
                            color: rgba(0x000000b3).into(),
                            offset: point(px(0.), px(24.)),
                            blur_radius: px(60.),
                            spread_radius: px(-18.),
                            inset: false,
                        }])
                })),
        )
}
