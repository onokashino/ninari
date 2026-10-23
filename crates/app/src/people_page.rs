//! A character's page and a person's: the portrait beside the names, then who voices the character
//! and their titles, or a voice actor's roles.

use std::sync::Arc;

use gpui::{
    AnyElement, App, Context, Div, EventEmitter, FontWeight, IntoElement, ObjectFit, Render,
    Window, div, img, prelude::*, px, relative, rgb,
};
use ninari_core::catalog::people::PersonPage as Person;
use ninari_core::feeds::{self, CharacterView};

use crate::cards::{CardText, poster_card_sized};
use crate::core::Core;
use crate::data::{Load, fetch};
use crate::facts::long_date;
use crate::library_page::{content_width, grid_columns};
use crate::nav::{PageEvent, Route};
use crate::theme::{self, Accent};
use crate::thumbs::remote_image;

/// A picture from Shikimori, `width` wide and filling `height`, over a blank until it is in.
fn photo(url: Option<&str>, width: f32, height: f32, radius: gpui::Pixels, cx: &mut App) -> Div {
    let path = url.and_then(|u| remote_image(u, cx));
    div()
        .flex_none()
        .w(px(width))
        .h(px(height))
        .rounded(radius)
        .bg(rgb(theme::SURFACE_2))
        .border_1()
        .border_color(theme::line())
        // The box's own ratio: otherwise GPUI gives the image its picture's proportions.
        .children(path.map(|p| {
            img(p)
                .size_full()
                .aspect_ratio(width / height)
                .rounded(radius)
                .object_fit(ObjectFit::Cover)
        }))
}

fn page() -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(48.))
        .px(px(48.))
        .pt(px(50.))
        .pb(px(72.))
}

/// The portrait and the names; `lines` are the quiet rows under the name.
fn head(
    image: Option<&str>,
    name: &str,
    lines: Vec<Vec<String>>,
    extra: Option<AnyElement>,
    cx: &mut App,
) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(32.))
        .child(photo(image, 200., 300., theme::R_L, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(46.))
                        .line_height(px(48.))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .child(name.to_string()),
                )
                .children(lines.into_iter().filter(|l| !l.is_empty()).map(|line| {
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_x(px(16.))
                        .gap_y(px(8.))
                        .text_color(rgb(theme::TEXT_2))
                        .font_weight(FontWeight::SEMIBOLD)
                        .children(line)
                }))
                .children(extra),
        )
}

/// The head while the page loads, or the error that stopped it.
fn waiting<T>(load: &Load<T>) -> Option<Div> {
    match load {
        Load::Pending => Some(
            page().child(
                div()
                    .flex()
                    .gap(px(32.))
                    .child(
                        div()
                            .w(px(200.))
                            .h(px(300.))
                            .rounded(theme::R_L)
                            .bg(theme::glass()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .gap(px(12.))
                            .child(
                                div()
                                    .w(relative(0.5))
                                    .h(px(44.))
                                    .rounded(px(10.))
                                    .bg(theme::glass()),
                            )
                            .child(
                                div()
                                    .w(relative(0.3))
                                    .h(px(20.))
                                    .rounded(px(8.))
                                    .bg(theme::glass()),
                            ),
                    ),
            ),
        ),
        Load::Failed(e) => {
            Some(page().child(div().text_color(rgb(theme::DANGER)).child(e.clone())))
        }
        Load::Ready(_) => None,
    }
}

fn heading(text: &'static str) -> Div {
    div()
        .text_size(px(18.))
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(text)
}

pub struct CharacterPage {
    view: Load<CharacterView>,
}

impl EventEmitter<PageEvent> for CharacterPage {}

impl CharacterPage {
    pub fn new(core: Arc<Core>, id: i64, cx: &mut Context<Self>) -> Self {
        fetch(
            cx,
            core.run(move |c| async move { feeds::character(&c.catalog, &c.posters, id).await }),
            |page, view| page.view = view,
        );
        Self {
            view: Load::Pending,
        }
    }
}

impl Render for CharacterPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(w) = waiting(&self.view) {
            return w;
        }
        let Load::Ready(v) = &self.view else {
            unreachable!()
        };
        let accent = Accent::colors(cx).0;
        let c = &v.page;
        let names: Vec<String> = [&c.original, &c.japanese, &c.altname]
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        let description = c.description.clone().filter(|d| !d.is_empty()).map(|d| {
            div()
                .mt(px(12.))
                .max_w(px(680.))
                .text_color(rgb(theme::TEXT_2))
                .line_height(px(24.))
                .child(d)
                .into_any_element()
        });
        let mut out = page().child(head(
            c.image.as_deref(),
            &c.name,
            vec![names],
            description,
            cx,
        ));

        if !c.voices.is_empty() {
            let voices: Vec<AnyElement> = c
                .voices
                .iter()
                .map(|p| {
                    let id = p.id;
                    div()
                        .id(("voice", id as u64))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .pl(px(6.))
                        .pr(px(14.))
                        .py(px(6.))
                        .rounded_full()
                        .bg(theme::glass())
                        .border_1()
                        .border_color(theme::line())
                        .font_weight(FontWeight::SEMIBOLD)
                        .hover(|s| s.bg(theme::glass_strong()))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(PageEvent::Open(Route::Person(id)))
                        }))
                        .child(photo(p.image.as_deref(), 36., 36., px(18.), cx))
                        .child(p.name.clone())
                        .into_any_element()
                })
                .collect();
            out = out.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(heading("Озвучивают"))
                    .child(div().flex().flex_wrap().gap(px(8.)).children(voices)),
            );
        }

        if !v.titles.is_empty() {
            let (_, card_w) = grid_columns(content_width(window));
            let cards: Vec<AnyElement> =
                v.titles
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        let id = t.card.anime.id;
                        let open = Box::new(cx.listener(move |_, _, _, cx| {
                            cx.emit(PageEvent::Open(Route::Title(id)))
                        }));
                        let text = CardText {
                            note: Some(
                                if t.main {
                                    "Главная роль"
                                } else {
                                    "Второстепенная роль"
                                }
                                .into(),
                            ),
                            fraction: None,
                            badge: None,
                        };
                        poster_card_sized(
                            &t.card,
                            "character",
                            i,
                            text,
                            card_w,
                            accent,
                            open,
                            window,
                            cx,
                        )
                        .into_any_element()
                    })
                    .collect();
            out = out.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(heading("Аниме"))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_x(px(24.))
                            .gap_y(px(32.))
                            .children(cards),
                    ),
            );
        }
        out
    }
}

fn open(route: Route, cx: &mut Context<PersonPage>) -> crate::OnClick {
    Box::new(cx.listener(move |_, _, _, cx| cx.emit(PageEvent::Open(route))))
}

pub struct PersonPage {
    person: Load<Person>,
}

impl EventEmitter<PageEvent> for PersonPage {}

impl PersonPage {
    pub fn new(core: Arc<Core>, id: i64, cx: &mut Context<Self>) -> Self {
        fetch(
            cx,
            core.run(move |c| async move { feeds::person(&c.catalog, id).await }),
            |page, person| page.person = person,
        );
        Self {
            person: Load::Pending,
        }
    }
}

impl Render for PersonPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(w) = waiting(&self.person) {
            return w;
        }
        let Load::Ready(p) = &self.person else {
            unreachable!()
        };
        let accent = Accent::colors(cx).0;
        let names: Vec<String> = [&p.original, &p.japanese]
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        let about: Vec<String> = [
            p.job.clone(),
            p.birth
                .as_deref()
                .and_then(long_date)
                .map(|b| format!("День рождения: {b}")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let mut out = page().child(head(
            p.image.as_deref(),
            &p.name,
            vec![names, about],
            None,
            cx,
        ));

        if !p.roles.is_empty() {
            // Columns from 140px with 16px between them, as `minmax(140px, 1fr)`.
            let width = content_width(window);
            let columns = ((width + 16.) / 156.).floor().max(1.);
            let w = ((width - 16. * (columns - 1.)) / columns).floor();
            let roles: Vec<AnyElement> = p
                .roles
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let (character, anime) = (r.character_id, r.anime_id);
                    div()
                        .w(px(w))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(
                            div()
                                .id(("role-photo", i))
                                .cursor_pointer()
                                .on_click(open(Route::Character(character), cx))
                                .child(photo(
                                    r.character_image.as_deref(),
                                    w,
                                    w * 1.5,
                                    theme::R_M,
                                    cx,
                                )),
                        )
                        .child(
                            div()
                                .id(("role-name", i))
                                .cursor_pointer()
                                .text_size(px(13.))
                                .line_height(px(17.))
                                .font_weight(FontWeight::BOLD)
                                .hover(move |s| s.text_color(accent))
                                .on_click(open(Route::Character(character), cx))
                                .child(r.character.clone()),
                        )
                        .child(
                            div()
                                .id(("role-title", i))
                                .cursor_pointer()
                                .text_size(px(12.))
                                .line_height(px(16.))
                                .text_color(rgb(theme::TEXT_3))
                                .line_clamp(2)
                                .text_ellipsis()
                                .hover(move |s| s.text_color(accent))
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(PageEvent::Open(Route::Title(anime)))
                                }))
                                .child(r.anime.clone()),
                        )
                        .into_any_element()
                })
                .collect();
            out = out.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(heading("Роли"))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_x(px(16.))
                            .gap_y(px(24.))
                            .children(roles),
                    ),
            );
        }
        out
    }
}
