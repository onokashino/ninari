//! Ctrl+K from anywhere: sections, the library's titles and the catalog, by keyboard.

use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, BoxShadow, Context, Entity, EventEmitter, FontWeight, IntoElement, KeyDownEvent,
    Render, Task, Window, div, point, prelude::*, px, rgb, rgba,
};
use ninari_core::feeds::{self, AnimeCard, LibraryItem};

use crate::core::Core;
use crate::data::{Load, fetch, title};
use crate::format::{kind_label, library_status};
use crate::library::query_variants;
use crate::nav::Route;
use crate::popover::close_on_escape;
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, Accent};

const PAGES: [(&str, Route, &str); 8] = [
    ("Главная", Route::Home, "продолжить новое сегодня"),
    ("Поиск", Route::Search, "найти кадр скриншот"),
    ("Каталог", Route::Catalog, "жанры сезоны подборки случайный"),
    (
        "Библиотека",
        Route::Library,
        "смотрю планы просмотрено брошено список",
    ),
    (
        "История",
        Route::History,
        "недавно смотрел просмотренные серии",
    ),
    ("Расписание", Route::Schedule, "выходит сегодня онгоинги"),
    ("Статистика", Route::Stats, "часы жанры студии"),
    (
        "Настройки",
        Route::Settings,
        "плагины источники устройства синхронизация данные апскейл обновления",
    ),
];
const MAX_TITLES: usize = 6;
const RECENT: usize = 5;
const DEBOUNCE: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub label: String,
    pub hint: String,
    pub route: Route,
}

fn norm(s: &str) -> String {
    s.to_lowercase().replace('ё', "е")
}

fn matches<'a>(texts: impl IntoIterator<Item = &'a str>, variants: &[String]) -> bool {
    let hay: Vec<String> = texts.into_iter().map(norm).collect();
    variants
        .iter()
        .any(|v| hay.iter().any(|h| h.contains(v.as_str())))
}

fn names(c: &AnimeCard) -> Vec<&str> {
    let a = &c.anime;
    [
        a.russian.as_deref(),
        Some(a.name.as_str()),
        a.english.as_deref(),
        a.japanese.as_deref(),
    ]
    .into_iter()
    .flatten()
    .chain(a.synonyms.iter().map(String::as_str))
    .collect()
}

/// What Ctrl+K offers for a query: sections, then the library's titles, then catalog finds the
/// library does not have. With nothing typed, the sections and the titles watched last.
pub fn palette_items(query: &str, library: &[LibraryItem], found: &[AnimeCard]) -> Vec<Item> {
    let variants: Vec<String> = query_variants(query).iter().map(|v| norm(v)).collect();
    let page = |(label, route, _): &(&str, Route, &str)| Item {
        label: label.to_string(),
        hint: "Раздел".into(),
        route: *route,
    };
    let own = |i: &LibraryItem| Item {
        label: title(&i.card.anime),
        hint: library_status(&i.entry.status).to_string(),
        route: Route::Title(i.card.anime.id),
    };
    if variants.is_empty() {
        let mut recent: Vec<&LibraryItem> = library.iter().collect();
        recent.sort_by_key(|i| std::cmp::Reverse(i.last_watched_at.unwrap_or(0)));
        return PAGES
            .iter()
            .map(page)
            .chain(recent.into_iter().take(RECENT).map(own))
            .collect();
    }
    let pages = PAGES
        .iter()
        .filter(|(label, _, keywords)| matches([*label, *keywords], &variants))
        .map(page);
    let mine = library
        .iter()
        .filter(|i| matches(names(&i.card), &variants))
        .take(MAX_TITLES)
        .map(own);
    let others = found
        .iter()
        .filter(|c| !library.iter().any(|i| i.card.anime.id == c.anime.id))
        .take(MAX_TITLES)
        .map(|c| {
            let a = &c.anime;
            let hint: Vec<String> = [
                Some(kind_label(a.kind.as_deref())).filter(|k| !k.is_empty()),
                a.year.map(|y| y.to_string()),
            ]
            .into_iter()
            .flatten()
            .collect();
            Item {
                label: title(a),
                hint: if hint.is_empty() {
                    "Каталог".into()
                } else {
                    hint.join(", ")
                },
                route: Route::Title(a.id),
            }
        });
    pages.chain(mine).chain(others).collect()
}

/// The palette wants to go somewhere, or to close.
pub enum PaletteEvent {
    Go(Route),
    Close,
}

pub struct CommandPalette {
    core: Arc<Core>,
    input: Entity<TextInput>,
    library: Vec<LibraryItem>,
    found: Vec<AnimeCard>,
    searching: bool,
    active: usize,
    debounce: Option<Task<()>>,
}

impl EventEmitter<PaletteEvent> for CommandPalette {}

impl CommandPalette {
    pub fn new(core: Arc<Core>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut input = TextInput::new("Раздел или тайтл", cx);
            input.compact = true;
            input
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.subscribe(&input, |palette, _, event: &InputEvent, cx| match event {
            InputEvent::Changed => palette.typed(cx),
            InputEvent::Submit => palette.go(palette.active, cx),
            InputEvent::Image(_) => {}
        })
        .detach();
        fetch(
            cx,
            core.run(|c| async move {
                feeds::library_items(&c.library, &c.catalog, &c.posters, None).await
            }),
            |palette, list| {
                if let Load::Ready(list) = list {
                    palette.library = list;
                }
            },
        );
        Self {
            core,
            input,
            library: Vec::new(),
            found: Vec::new(),
            searching: false,
            active: 0,
            debounce: None,
        }
    }

    fn query(&self, cx: &Context<Self>) -> String {
        self.input.read(cx).text().trim().to_string()
    }

    fn items(&self, cx: &Context<Self>) -> Vec<Item> {
        palette_items(&self.query(cx), &self.library, &self.found)
    }

    /// The catalog is asked once typing pauses.
    fn typed(&mut self, cx: &mut Context<Self>) {
        self.active = 0;
        let query = self.query(cx);
        if query.chars().count() < 2 {
            self.found.clear();
            self.debounce = None;
            self.searching = false;
            cx.notify();
            return;
        }
        self.searching = true;
        let core = self.core.clone();
        self.debounce = Some(cx.spawn(async move |palette, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            let job = core.run(move |c| async move {
                let list = c.catalog.search(&query, 30).await?;
                Ok(list
                    .into_iter()
                    .map(|a| feeds::card(&c.posters, a))
                    .collect::<Vec<_>>())
            });
            let got = job.await;
            palette
                .update(cx, |palette, cx| {
                    palette.searching = false;
                    palette.found = got.unwrap_or_default();
                    cx.notify();
                })
                .ok();
        }));
        cx.notify();
    }

    fn go(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(item) = self.items(cx).get(index) {
            cx.emit(PaletteEvent::Go(item.route));
        }
    }

    fn on_key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.items(cx).len().max(1);
        match e.keystroke.key.as_str() {
            "down" => self.active = (self.active + 1) % n,
            "up" => self.active = (self.active + n - 1) % n,
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for CommandPalette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        let items = self.items(cx);
        let current = self.active.min(items.len().saturating_sub(1));
        let me = cx.entity().downgrade();
        close_on_escape(cx, move |_, cx| {
            me.update(cx, |_, cx| cx.emit(PaletteEvent::Close)).ok();
        });
        let rows: Vec<AnyElement> = items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                div()
                    .id(("palette", i))
                    .cursor_pointer()
                    .flex()
                    .items_baseline()
                    .justify_between()
                    .gap(px(16.))
                    .px(px(12.))
                    .py(px(10.))
                    .rounded(theme::R_M)
                    .when(i == current, |d| d.bg(accent.opacity(0.18)))
                    .on_mouse_move(cx.listener(move |palette, _, _, cx| {
                        if palette.active != i {
                            palette.active = i;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |palette, _, _, cx| palette.go(i, cx)))
                    .child(
                        div()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(item.label.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(13.))
                            .text_color(rgb(theme::TEXT_3))
                            .child(item.hint.clone()),
                    )
                    .into_any_element()
            })
            .collect();
        let empty = rows.is_empty().then(|| {
            div()
                .px(px(12.))
                .py(px(14.))
                .text_color(rgb(theme::TEXT_3))
                .child(if self.searching {
                    "Ищу в каталоге…"
                } else {
                    "Ничего не нашлось"
                })
        });
        let view = window.viewport_size();
        let panel = div()
            .occlude()
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::Close)))
            .on_key_down(cx.listener(Self::on_key))
            .flex()
            .flex_col()
            .w(px(600.).min(view.width - px(48.)))
            .self_start()
            .rounded(theme::R_L)
            .bg(rgba(0x14141efa))
            .border_1()
            .border_color(rgba(0xffffff14))
            .shadow(vec![BoxShadow {
                color: rgba(0x00000099).into(),
                offset: point(px(0.), px(30.)),
                blur_radius: px(80.),
                spread_radius: px(0.),
                inset: false,
            }])
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .p(px(12.))
                    .border_b_1()
                    .border_color(theme::line())
                    .child(div().flex_1().child(self.input.clone()))
                    .child(
                        div()
                            .px(px(8.))
                            .py(px(2.))
                            .rounded(px(6.))
                            .bg(theme::glass_strong())
                            .text_size(px(12.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(theme::TEXT_3))
                            .child("Esc"),
                    ),
            )
            .child(
                div()
                    .id("palette-list")
                    .max_h((view.height * 0.52).min(px(440.)))
                    .overflow_y_scroll()
                    .p(px(8.))
                    .children(rows)
                    .children(empty),
            );
        // Near the top, as the place a search starts from; the page behind is only dimmed.
        div()
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            .pt(view.height * 0.14)
            .px(px(24.))
            .bg(rgba(0x08080c8c))
            .child(panel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ninari_core::catalog::Anime;
    use ninari_core::library::LibraryEntry;

    fn card(id: i64, russian: &str, name: &str, synonyms: &[&str]) -> AnimeCard {
        AnimeCard {
            anime: Anime {
                id,
                name: name.into(),
                russian: Some(russian.into()),
                english: None,
                japanese: None,
                synonyms: synonyms.iter().map(|s| s.to_string()).collect(),
                kind: Some("tv".into()),
                status: None,
                episodes: 12,
                episodes_aired: 12,
                year: Some(2023),
                next_episode_at: None,
                poster_url: None,
                genres: vec![],
                studios: vec![],
                studio_ids: vec![],
                description: None,
            },
            poster_path: None,
            palette: None,
        }
    }

    fn item(card: AnimeCard, watched_at: i64) -> LibraryItem {
        LibraryItem {
            entry: LibraryEntry {
                shikimori_id: card.anime.id,
                status: "watching".into(),
                chosen_dub_key: None,
                notify: true,
                added_at: 0,
                updated_at: 0,
                score: None,
                note: None,
            },
            card,
            watched: 0,
            last_watched_at: Some(watched_at),
        }
    }

    const FRIEREN: &str = "Провожающая в последний путь Фрирен";
    const THRONE: &str = "Трон, отмеченный богом 2";

    fn library() -> Vec<LibraryItem> {
        vec![
            item(card(52991, FRIEREN, "Sousou no Frieren", &["Frieren"]), 100),
            item(card(52684, THRONE, "Shen Yin Wangzuo 2nd Season", &[]), 200),
        ]
    }

    fn labels(q: &str) -> Vec<String> {
        palette_items(q, &library(), &[])
            .into_iter()
            .map(|i| i.label)
            .collect()
    }

    #[test]
    fn with_nothing_typed_sections_then_what_was_watched_last() {
        let items = labels("");
        let pages: Vec<&str> = PAGES.iter().map(|(l, ..)| *l).collect();
        assert_eq!(&items[..PAGES.len()], pages.as_slice());
        assert_eq!(&items[PAGES.len()..], [THRONE, FRIEREN]);
    }

    #[test]
    fn titles_match_by_any_name_and_sections_by_keyword() {
        assert_eq!(labels("фрир"), [FRIEREN]);
        assert_eq!(labels("sousou"), [FRIEREN]);
        assert_eq!(labels("настр"), ["Настройки"]);
        assert_eq!(labels("плагин"), ["Настройки"]);
        assert_eq!(labels("отмеченный"), [THRONE]);
        assert_eq!(labels("ahbh"), [FRIEREN]);
    }

    #[test]
    fn catalog_finds_come_after_the_library_without_repeating_it() {
        let monster = card(19, "Монстр", "Monster", &[]);
        let frieren = card(52991, FRIEREN, "Sousou no Frieren", &[]);
        let items = palette_items("мон", &library(), &[monster, frieren]);
        assert_eq!(
            items,
            [Item {
                label: "Монстр".into(),
                hint: "Сериал, 2023".into(),
                route: Route::Title(19),
            }]
        );
    }
}
