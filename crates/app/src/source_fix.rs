//! "Не тот тайтл?": every source with what it matched for a title, and ways to correct a wrong
//! match: another candidate, a search by hand, or "this source has no such title".

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, Div, Entity, EventEmitter, FontWeight, Hsla, IntoElement, Render, Window,
    div, prelude::*, px, rgb,
};
use ninari_core::extensions::types::Match;
use ninari_core::sources::SourceMatch;

use crate::assets::icon;
use crate::button::{Size, Variant, button};
use crate::core::Core;
use crate::data::{Load, fetch};
use crate::modal::modal;
use crate::settings_page::muted;
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, Accent};

pub enum FixEvent {
    /// A match changed: the title's sources are found again.
    Changed,
    Closed,
}

fn state(m: &SourceMatch) -> &'static str {
    match (&m.matched, m.pinned) {
        (Some(_), true) => "выбрано вручную",
        (Some(found), false) if found.confidence == "exact" => "точно",
        (Some(_), false) => "похоже",
        (None, true) => "отмечено, что тайтла нет",
        (None, false) => "не найдено",
    }
}

pub struct SourceFix {
    core: Arc<Core>,
    id: i64,
    matches: Load<Vec<SourceMatch>>,
    /// The source whose picker is open.
    picking: Option<String>,
    query: Entity<TextInput>,
    found: Option<Load<Vec<Match>>>,
    error: Option<String>,
}

impl EventEmitter<FixEvent> for SourceFix {}

impl SourceFix {
    pub fn new(core: Arc<Core>, id: i64, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| {
            let mut input = TextInput::new("Название, как на сайте источника", cx);
            input.compact = true;
            input.search = false;
            input
        });
        cx.subscribe(&query, |fix, _, event: &InputEvent, cx| match event {
            InputEvent::Submit => fix.search(cx),
            _ => cx.notify(),
        })
        .detach();
        let mut fix = Self {
            core,
            id,
            matches: Load::Pending,
            picking: None,
            query,
            found: None,
            error: None,
        };
        fix.reload(cx);
        fix
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let id = self.id;
        fetch(
            cx,
            self.core
                .run(move |c| async move { c.resolver.matches(id) }),
            |fix, matches| fix.matches = matches,
        );
    }

    /// `chosen` for that source, or None for "it has no such title".
    fn pin(&mut self, ext: String, chosen: Option<Match>, cx: &mut Context<Self>) {
        let id = self.id;
        let job = self
            .core
            .run(move |c| async move { c.resolver.pin_match(id, &ext, chosen) });
        self.after(job, cx);
    }

    fn unpin(&mut self, ext: String, cx: &mut Context<Self>) {
        let id = self.id;
        let job = self
            .core
            .run(move |c| async move { c.resolver.unpin(id, &ext) });
        self.after(job, cx);
    }

    fn after(
        &mut self,
        job: impl Future<Output = anyhow::Result<()>> + 'static,
        cx: &mut Context<Self>,
    ) {
        self.error = None;
        cx.spawn(async move |fix, cx| {
            let got = job.await;
            fix.update(cx, |fix, cx| {
                match got {
                    Ok(()) => {
                        fix.picking = None;
                        fix.found = None;
                        cx.emit(FixEvent::Changed);
                    }
                    Err(e) => fix.error = Some(format!("{e:#}")),
                }
                fix.reload(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        let (Some(ext), query) = (
            self.picking.clone(),
            self.query.read(cx).text().trim().to_string(),
        ) else {
            return;
        };
        if query.is_empty() {
            return;
        }
        let id = self.id;
        self.found = Some(Load::Pending);
        fetch(
            cx,
            self.core
                .run(move |c| async move { c.resolver.search_in(id, &ext, &query).await }),
            |fix, found| fix.found = Some(found),
        );
        cx.notify();
    }

    fn candidates(
        &self,
        key: &'static str,
        ext: &str,
        items: &[Match],
        current: Option<&str>,
        accent: Hsla,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .children(items.iter().enumerate().map(|(i, c)| {
                let (ext, chosen) = (ext.to_string(), c.clone());
                let picked = current == Some(c.id.as_str());
                div()
                    .id((key, i))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(10.))
                    .text_size(px(13.))
                    .hover(|s| s.bg(theme::glass_strong()))
                    .on_click(cx.listener(move |fix, _, _, cx| {
                        fix.pin(ext.clone(), Some(chosen.clone()), cx)
                    }))
                    .child(c.title.clone())
                    .when(picked, |d| d.child(icon("check", 16.).text_color(accent)))
            }))
    }

    fn row(&self, m: &SourceMatch, i: usize, accent: Hsla, cx: &mut Context<Self>) -> AnyElement {
        let open = self.picking.as_deref() == Some(m.extension_id.as_str());
        let link = |id: (&'static str, usize), label: &'static str| {
            div()
                .id(id)
                .cursor_pointer()
                .text_size(px(13.))
                .font_weight(FontWeight::BOLD)
                .text_color(accent)
                .hover(|s| s.underline())
                .child(label)
        };
        let (pick_ext, none_ext, reset_ext) = (
            m.extension_id.clone(),
            m.extension_id.clone(),
            m.extension_id.clone(),
        );
        let mut actions = vec![
            link(("pick", i), "Выбрать другое")
                .on_click(cx.listener(move |fix, _, _, cx| {
                    fix.picking = if fix.picking.as_deref() == Some(pick_ext.as_str()) {
                        None
                    } else {
                        Some(pick_ext.clone())
                    };
                    fix.found = None;
                    cx.notify();
                }))
                .into_any_element(),
        ];
        if !(m.pinned && m.matched.is_none()) {
            actions.push(
                link(("none", i), "Нет в этом источнике")
                    .on_click(cx.listener(move |fix, _, _, cx| fix.pin(none_ext.clone(), None, cx)))
                    .into_any_element(),
            );
        }
        if m.pinned {
            actions.push(
                link(("reset", i), "Сбросить")
                    .on_click(cx.listener(move |fix, _, _, cx| fix.unpin(reset_ext.clone(), cx)))
                    .into_any_element(),
            );
        }
        let current = m.matched.as_ref().map(|c| c.id.as_str());
        let picker = open.then(|| {
            let found: Option<AnyElement> = match &self.found {
                Some(Load::Pending) => Some(muted("Ищу…").into_any_element()),
                Some(Load::Failed(e)) => Some(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(theme::DANGER))
                        .child(e.clone())
                        .into_any_element(),
                ),
                Some(Load::Ready(list)) if list.is_empty() => {
                    Some(muted("Ничего не нашлось.").into_any_element())
                }
                Some(Load::Ready(list)) => Some(
                    self.candidates("found", &m.extension_id, list, current, accent, cx)
                        .into_any_element(),
                ),
                None => None,
            };
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .when(!m.candidates.is_empty(), |d| {
                    d.child(self.candidates(
                        "candidate",
                        &m.extension_id,
                        &m.candidates,
                        current,
                        accent,
                        cx,
                    ))
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().flex_1().child(self.query.clone()))
                        .child(
                            button("find", "Найти", None, Variant::Glass, Size::M, accent)
                                .on_click(cx.listener(|fix, _, _, cx| fix.search(cx))),
                        ),
                )
                .children(found)
        });
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(16.))
            .py(px(12.))
            .rounded(theme::R_M)
            .bg(theme::glass())
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_baseline()
                    .gap_x(px(12.))
                    .gap_y(px(4.))
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .child(m.extension_name.clone()),
                    )
                    .children(m.matched.as_ref().map(|c| {
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(theme::TEXT_2))
                            .child(c.title.clone())
                    }))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(if m.pinned {
                                accent
                            } else {
                                rgb(theme::TEXT_3).into()
                            })
                            .child(state(m)),
                    ),
            )
            .child(div().flex().flex_wrap().gap(px(16.)).children(actions))
            .children(picker)
            .into_any_element()
    }
}

impl Render for SourceFix {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        let list: AnyElement = match &self.matches {
            Load::Pending => div()
                .h(px(160.))
                .rounded(theme::R_M)
                .bg(theme::glass())
                .into_any_element(),
            Load::Failed(e) => div()
                .text_color(rgb(theme::DANGER))
                .child(e.clone())
                .into_any_element(),
            Load::Ready(matches) => {
                let rows: Vec<AnyElement> = matches
                    .iter()
                    .enumerate()
                    .map(|(i, m)| self.row(m, i, accent, cx))
                    .collect();
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .children(rows)
                    .into_any_element()
            }
        };
        let body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(muted(
                "Если источник подобрал чужое аниме, выбери правильное из найденного, найди вручную \
                 или отметь, что там этого тайтла нет.",
            ))
            .child(list)
            .children(self.error.clone().map(|e| {
                div()
                    .text_size(px(13.))
                    .text_color(rgb(theme::DANGER))
                    .child(e)
            }))
            .into_any_element();
        let me = cx.entity().downgrade();
        div().children(modal(
            "source-fix",
            true,
            "Источники этого тайтла",
            body,
            Rc::new(move |_, cx| {
                me.update(cx, |_, cx| cx.emit(FixEvent::Closed)).ok();
            }),
            window,
            cx,
        ))
    }
}
