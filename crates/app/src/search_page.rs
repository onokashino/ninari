//! Search: titles by name, in either keyboard layout; recent queries; the season's popular titles
//! while nothing is typed; and search by a frame from an episode through trace.moe: a file, a
//! pasted picture or one dropped on the window.

use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, ClipboardEntry, Context, Entity, EventEmitter, ExternalPaths, FocusHandle,
    FontWeight, IntoElement, KeyDownEvent, ObjectFit, PathPromptOptions, Render, Task, Window, div,
    img, prelude::*, px, rgb,
};
use ninari_core::catalog::trace;
use ninari_core::db;
use ninari_core::feeds::{self, AnimeCard};
use serde_json::json;

use crate::assets::icon;
use crate::button::{Size, Variant, button};
use crate::cards::{CardText, poster_card_sized};
use crate::core::Core;
use crate::data::{Load, fetch, title};
use crate::episode_grid::chip;
use crate::library::swap_layout;
use crate::library_page::{Open, content_width, grid_columns, more_link};
use crate::nav::Route;
use crate::player::{Request, clock};
use crate::posters::poster;
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, Accent};
use crate::thumbs::remote_image;

/// Where the search history is kept.
const HISTORY_SETTING: &str = "search.history";
const HISTORY_SIZE: usize = 8;
/// Longest side of the frame sent to trace.moe: enough to match, small to send.
const FRAME_SIDE: u32 = 640;

/// The history with `query` first; the same query in another case replaces the old one.
pub fn remember(history: &[String], query: &str) -> Vec<String> {
    let q = query.trim();
    if q.is_empty() {
        return history.to_vec();
    }
    std::iter::once(q.to_string())
        .chain(
            history
                .iter()
                .filter(|h| h.to_lowercase() != q.to_lowercase())
                .cloned(),
        )
        .take(HISTORY_SIZE)
        .collect()
}

/// A screenshot shrunk to a small JPEG.
pub fn frame_jpeg(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let picture = image::load_from_memory(bytes)?;
    let picture = if picture.width().max(picture.height()) > FRAME_SIDE {
        picture.resize(
            FRAME_SIDE,
            FRAME_SIDE,
            image::imageops::FilterType::Triangle,
        )
    } else {
        picture
    };
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
        .encode_image(&picture.to_rgb8())?;
    Ok(out)
}

pub enum SearchEvent {
    Open(Route),
    Play(Request),
}

pub struct FrameHit {
    card: AnimeCard,
    episode: Option<i64>,
    at: f64,
    similarity: f64,
    image: Option<String>,
    sure: bool,
}

pub struct SearchPage {
    core: Arc<Core>,
    input: Entity<TextInput>,
    focus: FocusHandle,
    /// The query being shown, the input's text a moment after typing stops.
    query: String,
    debounce: Option<Task<()>>,
    /// What was found and for which query (the other layout's, when only that found anything).
    results: Load<(Vec<AnimeCard>, String)>,
    history: Vec<String>,
    popular: Load<Vec<AnimeCard>>,
    frame: Option<Load<Vec<FrameHit>>>,
}

impl EventEmitter<SearchEvent> for SearchPage {}
impl EventEmitter<Open> for SearchPage {}

impl SearchPage {
    pub fn new(core: Arc<Core>, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| TextInput::new("Название по-русски, по-английски или по-японски", cx));
        cx.subscribe(&input, |page, input, event: &InputEvent, cx| match event {
            InputEvent::Changed => {
                let text = input.read(cx).text().trim().to_string();
                page.debounce = Some(cx.spawn(async move |page, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(300))
                        .await;
                    page.update(cx, |page, cx| page.search(text, cx)).ok();
                }));
            }
            InputEvent::Submit => {
                let text = input.read(cx).text().trim().to_string();
                page.debounce = None;
                page.search(text, cx);
            }
            InputEvent::Image(bytes) => page.find_frame(bytes.clone(), cx),
        })
        .detach();
        let page = Self {
            core,
            input,
            focus: cx.focus_handle(),
            query: String::new(),
            debounce: None,
            results: Load::Pending,
            history: Vec::new(),
            popular: Load::Pending,
            frame: None,
        };
        let db = page.core.db.clone();
        fetch(
            cx,
            page.core.run(move |_| async move {
                let saved = db::get_setting(&db.lock().unwrap(), HISTORY_SETTING)
                    .ok()
                    .flatten();
                Ok(saved
                    .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
                    .unwrap_or_default())
            }),
            |page, history| {
                if let Load::Ready(h) = history {
                    page.history = h;
                }
            },
        );
        fetch(
            cx,
            page.core.run(|c| async move {
                let list = c.catalog.season_top(20).await?;
                Ok(list
                    .into_iter()
                    .map(|a| feeds::card(&c.posters, a))
                    .collect())
            }),
            |page, popular| page.popular = popular,
        );
        page
    }

    /// The field takes the keyboard when the page opens.
    pub fn focus(&self, window: &mut Window, cx: &mut gpui::App) {
        let handle = gpui::Focusable::focus_handle(self.input.read(cx), cx);
        window.focus(&handle, cx);
    }

    /// Puts `text` in the field and searches it at once.
    pub fn set_query(&mut self, text: &str, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, cx| input.set_text(text.to_string(), cx));
        self.debounce = None;
        self.search(text.trim().to_string(), cx);
    }

    fn search(&mut self, query: String, cx: &mut Context<Self>) {
        if query == self.query {
            return;
        }
        self.query = query.clone();
        if query.is_empty() {
            cx.notify();
            return;
        }
        self.results = Load::Pending;
        let swapped = swap_layout(&query);
        fetch(
            cx,
            self.core.run(move |c| async move {
                let found = c.catalog.search(&query, 30).await?;
                // Nothing: perhaps it was typed with the other keyboard layout on.
                let (found, used) = if found.is_empty() && swapped != query {
                    (c.catalog.search(&swapped, 30).await?, swapped)
                } else {
                    (found, query)
                };
                Ok((
                    found
                        .into_iter()
                        .map(|a| feeds::card(&c.posters, a))
                        .collect(),
                    used,
                ))
            }),
            |page, results| page.results = results,
        );
        cx.notify();
    }

    fn save_history(&mut self, history: Vec<String>, cx: &mut Context<Self>) {
        self.history = history.clone();
        let db = self.core.db.clone();
        let job = self.core.run(move |_| async move {
            db::set_setting(&db.lock().unwrap(), HISTORY_SETTING, &json!(history))
                .map_err(|e| ninari_core::error::AppError::new("Settings", e.to_string()))
        });
        cx.background_spawn(async move {
            if let Err(e) = job.await {
                tracing::warn!("search history: {e:#}");
            }
        })
        .detach();
        cx.notify();
    }

    fn find_frame(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        self.frame = Some(Load::Pending);
        let jpeg = match frame_jpeg(&bytes) {
            Ok(j) => j,
            Err(e) => {
                self.frame = Some(Load::Failed(format!("не удалось прочитать картинку: {e}")));
                cx.notify();
                return;
            }
        };
        fetch(
            cx,
            self.core.run(move |c| async move {
                let hits = trace::search(&c.http, jpeg).await?;
                let ids: Vec<i64> = hits.iter().map(|h| h.mal_id).collect();
                let titles = c.catalog.get_many(&ids).await?;
                tracing::info!(
                    "frame search: {} matches over the bar {ids:?}, {} found in the catalogue",
                    hits.len(),
                    titles.len()
                );
                Ok(hits
                    .into_iter()
                    .filter_map(|h| {
                        let anime = titles.iter().find(|a| a.id == h.mal_id)?.clone();
                        Some(FrameHit {
                            card: feeds::card(&c.posters, anime),
                            episode: h.episode,
                            at: h.at,
                            similarity: h.similarity,
                            image: h.image,
                            sure: h.sure,
                        })
                    })
                    .collect())
            }),
            |page, hits| page.frame = Some(hits),
        );
        cx.notify();
    }

    fn pick_frame(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Кадр".into()),
        });
        cx.spawn(async move |page, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(bytes) = paths.first().and_then(|p| std::fs::read(p).ok()) else {
                return;
            };
            page.update(cx, |page, cx| page.find_frame(bytes, cx)).ok();
        })
        .detach();
    }

    fn open(&self, route: Route, cx: &mut Context<Self>) -> crate::OnClick {
        Box::new(cx.listener(move |_, _, _, cx| cx.emit(Open(route))))
    }

    fn grid(
        &self,
        list: &[AnimeCard],
        name: &'static str,
        remember_query: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = Accent::colors(cx).0;
        let (_, card_w) = grid_columns(content_width(window));
        let cards: Vec<AnyElement> = list
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let id = c.anime.id;
                let query = remember_query.clone();
                // Opening a result is what makes a query worth remembering.
                let open: crate::OnClick = Box::new(cx.listener(move |page, _, _, cx| {
                    if let Some(q) = &query {
                        let next = remember(&page.history, q);
                        page.save_history(next, cx);
                    }
                    cx.emit(Open(Route::Title(id)));
                }));
                let text = CardText {
                    note: None,
                    fraction: None,
                    badge: None,
                };
                poster_card_sized(c, name, i, text, card_w, accent, open, window, cx)
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_wrap()
            .gap_x(px(24.))
            .gap_y(px(32.))
            .children(cards)
            .into_any_element()
    }

    fn frame_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let accent = Accent::colors(cx).0;
        let busy = matches!(self.frame, Some(Load::Pending));
        let tools = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(12.))
            .child(
                button(
                    "frame",
                    if busy {
                        "Ищем кадр…"
                    } else {
                        "Найти по кадру"
                    },
                    Some("sparkle"),
                    Variant::Glass,
                    Size::M,
                    accent,
                )
                .when(!busy, |b| {
                    b.on_click(cx.listener(|page, _, _, cx| page.pick_frame(cx)))
                }),
            )
            .child(
                div()
                    .text_color(rgb(theme::TEXT_3))
                    .text_size(px(13.))
                    .child("или вставь скриншот через Ctrl+V, или перетащи его сюда"),
            );
        let result: Option<AnyElement> = match &self.frame {
            Some(Load::Failed(e)) => Some(
                div()
                    .text_color(rgb(theme::DANGER))
                    .text_size(px(13.))
                    .child(e.clone())
                    .into_any_element(),
            ),
            Some(Load::Ready(hits)) if hits.is_empty() => Some(
                div()
                    .text_color(rgb(theme::TEXT_3))
                    .text_size(px(13.))
                    .child(
                        "Похожего кадра не нашлось. Нужен кадр из самой серии: постеры и арты \
                         trace.moe не знает, а совсем новые серии появляются у него не сразу.",
                    )
                    .into_any_element(),
            ),
            Some(Load::Ready(hits)) => Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child(if hits.iter().any(|h| h.sure) {
                                "Нашлось по кадру"
                            } else {
                                "Может быть, это"
                            }),
                    )
                    .when(hits.iter().all(|h| !h.sure), |d| {
                        d.child(
                            div()
                                .mt(px(-8.))
                                .text_color(rgb(theme::TEXT_3))
                                .text_size(px(13.))
                                .child("Уверенных совпадений нет, вот самые похожие."),
                        )
                    })
                    .children(hits.iter().map(|h| self.frame_hit(h, accent, cx)))
                    .into_any_element(),
            ),
            _ => None,
        };
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(tools)
            .children(result)
            .into_any_element()
    }

    fn frame_hit(&self, h: &FrameHit, accent: gpui::Hsla, cx: &mut Context<Self>) -> AnyElement {
        let name = title(&h.card.anime);
        let at = clock(h.at);
        let still = h.image.as_deref().and_then(|url| remote_image(url, cx));
        let id = h.card.anime.id;
        let line = match h.episode {
            Some(ep) => format!(
                "Серия {ep}, {at}, совпадение {}%",
                (h.similarity * 100.).round()
            ),
            None => format!("{at}, совпадение {}%", (h.similarity * 100.).round()),
        };
        let watch = h.episode.map(|episode| {
            let request = Request {
                id,
                episode,
                dub_key: None,
                start: Some(h.at.floor()),
                title: name.clone().into(),
                palette: poster(&h.card, cx).map(|(_, p)| p).unwrap_or_default(),
            };
            div().flex().child(
                button(
                    ("frame-watch", id as u64),
                    format!("Смотреть с {at}"),
                    Some("play"),
                    Variant::Primary,
                    Size::M,
                    accent,
                )
                .on_click(
                    cx.listener(move |_, _, _, cx| cx.emit(SearchEvent::Play(request.clone()))),
                ),
            )
        });
        div()
            .flex()
            .items_center()
            .gap(px(24.))
            .p(px(12.))
            .rounded(theme::R_L)
            .bg(theme::glass())
            .child(
                div()
                    .flex_none()
                    .w(px(224.))
                    .h(px(126.))
                    .rounded(theme::R_M)
                    .overflow_hidden()
                    .bg(theme::glass_strong())
                    .children(still.map(|p| {
                        img(p)
                            .size_full()
                            .rounded(theme::R_M)
                            .object_fit(ObjectFit::Cover)
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .min_w(px(0.))
                    .child(
                        div()
                            .id(("frame-title", id as u64))
                            .cursor_pointer()
                            .text_size(px(18.))
                            .font_weight(FontWeight::BOLD)
                            .hover(move |s| s.text_color(accent))
                            .on_click(self.open(Route::Title(id), cx))
                            .child(name),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_3))
                            .text_size(px(13.))
                            .child(line),
                    )
                    .children(watch),
            )
            .into_any_element()
    }
}

impl Render for SearchPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        let mut sections: Vec<AnyElement> = Vec::new();
        if self.query.is_empty() {
            if !self.history.is_empty() {
                let chips = self.history.iter().map(|h| {
                    let (query, forget) = (h.clone(), h.clone());
                    div()
                        .relative()
                        .flex()
                        .child(
                            chip(gpui::SharedString::from(format!("q-{h}")), false, accent)
                                .pr(px(32.))
                                .on_click(
                                    cx.listener(move |page, _, _, cx| page.set_query(&query, cx)),
                                )
                                .child(h.clone()),
                        )
                        .child(
                            div()
                                .id(gpui::SharedString::from(format!("forget-{h}")))
                                .cursor_pointer()
                                .absolute()
                                .right(px(6.))
                                .top(px(7.))
                                .size(px(22.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(|s| s.bg(theme::glass_strong()))
                                .on_click(cx.listener(move |page, _, _, cx| {
                                    let next: Vec<String> = page
                                        .history
                                        .iter()
                                        .filter(|x| **x != forget)
                                        .cloned()
                                        .collect();
                                    page.save_history(next, cx);
                                }))
                                .child(icon("close", 14.).text_color(rgb(theme::TEXT_3))),
                        )
                });
                sections.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(16.))
                        .child(
                            div()
                                .flex()
                                .items_baseline()
                                .justify_between()
                                .child(
                                    div()
                                        .text_size(px(18.))
                                        .font_weight(FontWeight::EXTRA_BOLD)
                                        .child("Недавние запросы"),
                                )
                                .child(more_link(
                                    "clear-history",
                                    "Очистить",
                                    accent,
                                    Box::new(cx.listener(|page, _, _, cx| {
                                        page.save_history(Vec::new(), cx)
                                    })),
                                )),
                        )
                        .child(div().flex().flex_wrap().gap(px(8.)).children(chips))
                        .into_any_element(),
                );
            }
            if let Load::Ready(list) = &self.popular
                && !list.is_empty()
            {
                let list: Vec<AnimeCard> = list.iter().take(12).cloned().collect();
                let grid = self.grid(&list, "popular", None, window, cx);
                sections.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(16.))
                        .child(
                            div()
                                .text_size(px(18.))
                                .font_weight(FontWeight::EXTRA_BOLD)
                                .child("Популярное сейчас"),
                        )
                        .child(grid)
                        .into_any_element(),
                );
            }
        } else {
            let q = self.query.clone();
            match &self.results {
                Load::Pending => {
                    let (_, card_w) = grid_columns(content_width(window));
                    sections.push(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_x(px(24.))
                            .gap_y(px(32.))
                            .children((0..12).map(|_| {
                                div()
                                    .w(px(card_w))
                                    .h(px(card_w * 1.5))
                                    .rounded(theme::R_M)
                                    .bg(theme::glass())
                            }))
                            .into_any_element(),
                    );
                }
                Load::Failed(e) => sections.push(
                    div()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(px(16.))
                        .child(
                            div()
                                .text_color(rgb(theme::DANGER))
                                .child(format!("Каталог не ответил: {e}")),
                        )
                        .child(
                            button("retry-search", "Повторить", Some("retry"), Variant::Glass, Size::M, accent)
                                .on_click(cx.listener(|page, _, _, cx| {
                                    let q = std::mem::take(&mut page.query);
                                    page.search(q, cx);
                                })),
                        )
                        .into_any_element(),
                ),
                Load::Ready((list, _)) if list.is_empty() => sections.push(
                    div()
                        .text_color(rgb(theme::TEXT_2))
                        .child(format!(
                            "По запросу «{q}» ничего не нашлось. Попробуй другое написание или оригинальное название."
                        ))
                        .into_any_element(),
                ),
                Load::Ready((list, used)) => {
                    let (list, used) = (list.clone(), used.clone());
                    if used != q {
                        sections.push(
                            div()
                                .text_color(rgb(theme::TEXT_2))
                                .child(format!("По «{q}» ничего нет, показаны результаты для «{used}»."))
                                .into_any_element(),
                        );
                    }
                    sections.push(self.grid(&list, "results", Some(used), window, cx));
                }
            }
        }

        let frame = self.frame_section(cx);
        div()
            .id("search-page")
            // A click on the page gives it the keys, so Ctrl+V with a picture searches by it
            // wherever it is pressed, as the hint says; the field takes them while it has focus.
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|page, e: &KeyDownEvent, window, cx| {
                let k = &e.keystroke;
                if k.key != "v"
                    || !k.modifiers.control
                    || k.modifiers.alt
                    || page.input.read(cx).is_focused(window)
                {
                    return;
                }
                let image = cx.read_from_clipboard().and_then(|item| {
                    item.entries().iter().find_map(|e| match e {
                        ClipboardEntry::Image(image) => Some(image.bytes.clone()),
                        _ => None,
                    })
                });
                if let Some(bytes) = image {
                    page.find_frame(bytes, cx);
                }
            }))
            .on_drop(cx.listener(|page, paths: &ExternalPaths, _, cx| {
                if let Some(bytes) = paths.paths().first().and_then(|p| std::fs::read(p).ok()) {
                    page.find_frame(bytes, cx);
                }
            }))
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
                    .child("Поиск"),
            )
            .child(div().max_w(px(560.)).child(self.input.clone()))
            .child(frame)
            .children(sections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_goes_first_and_replaces_its_other_case() {
        let h = vec!["Фрирен".to_string(), "naruto".into()];
        assert_eq!(remember(&h, "NARUTO"), ["NARUTO", "Фрирен"]);
        assert_eq!(remember(&h, "  "), h);
        let long: Vec<String> = (0..8).map(|i| i.to_string()).collect();
        assert_eq!(remember(&long, "x").len(), 8);
    }

    #[test]
    fn a_frame_is_shrunk_to_a_small_jpeg() {
        let big = image::RgbImage::new(1920, 1080);
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(big)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let jpeg = frame_jpeg(&png).unwrap();
        let back = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((back.width(), back.height()), (640, 360));
    }
}
