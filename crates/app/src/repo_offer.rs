//! A plugin repository offered by an `ninari://` link from the website. A link alone installs
//! nothing: the repository, its plugins and the sites they reach are shown first.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, EventEmitter, FontWeight, IntoElement, Render, Window, div, prelude::*,
    px, rgb,
};

use crate::button::{Size, Variant, button};
use crate::core::Core;
use crate::data::{Load, fetch};
use crate::format::plural;
use crate::modal::modal;
use crate::nav::PageEvent;
use crate::settings_page::muted;
use crate::settings_plugins::{Registry, hosts_of, install_repo, preview_repo};
use crate::theme::{self, Accent};

/// The offer is done with: installed or turned down.
pub struct Closed;

pub struct RepoOffer {
    core: Arc<Core>,
    repo: String,
    preview: Load<Registry>,
    open: bool,
    installing: bool,
    error: Option<String>,
}

impl EventEmitter<Closed> for RepoOffer {}
impl EventEmitter<PageEvent> for RepoOffer {}

impl RepoOffer {
    pub fn new(core: Arc<Core>, repo: String, cx: &mut Context<Self>) -> Self {
        fetch(cx, preview_repo(&core, repo.clone()), |offer, preview| {
            offer.preview = preview
        });
        Self {
            core,
            repo,
            preview: Load::Pending,
            open: true,
            installing: false,
            error: None,
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        cx.emit(Closed);
        cx.notify();
    }

    fn install(&mut self, cx: &mut Context<Self>) {
        if self.installing {
            return;
        }
        self.installing = true;
        self.error = None;
        let job = install_repo(&self.core, self.repo.clone());
        cx.spawn(async move |offer, cx| {
            let got = job.await;
            offer
                .update(cx, |offer, cx| {
                    offer.installing = false;
                    match got {
                        Ok(()) => {
                            cx.emit(PageEvent::Changed);
                            offer.close(cx);
                        }
                        Err(e) => offer.error = Some(format!("{e:#}")),
                    }
                    cx.notify();
                })
                .ok();
        })
        .detach();
        cx.notify();
    }
}

impl Render for RepoOffer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        let host = url::Url::parse(&self.repo)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| self.repo.clone());
        let error = |text: String| {
            div()
                .text_size(px(13.))
                .text_color(rgb(theme::DANGER))
                .child(text)
                .into_any_element()
        };
        let mut parts: Vec<AnyElement> = vec![
            muted(format!(
                "Ссылка с сайта предлагает плагины из репозитория {host}."
            ))
            .into_any_element(),
        ];
        let offered = match &self.preview {
            Load::Pending => {
                parts.push(muted("Загружаю список…").into_any_element());
                0
            }
            Load::Failed(e) => {
                parts.push(error(e.clone()));
                0
            }
            Load::Ready(r) => {
                parts.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .children(r.entries.iter().map(|e| {
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(8.))
                                .child(div().font_weight(FontWeight::BOLD).child(e.name.clone()))
                                .child(muted(format!("v{}", e.version)))
                                .children(hosts_of(&e.domains, &e.allowed_hosts).into_iter().map(
                                    |h| {
                                        div()
                                            .px(px(8.))
                                            .py(px(1.))
                                            .rounded_full()
                                            .bg(theme::glass_strong())
                                            .text_size(px(12.))
                                            .child(h)
                                    },
                                ))
                        }))
                        .into_any_element(),
                );
                parts.push(
                    muted("Каждый плагин работает в песочнице и ходит только на свои сайты.")
                        .into_any_element(),
                );
                r.entries.len() as i64
            }
        };
        parts.extend(self.error.clone().map(error));
        parts.push(
            div()
                .flex()
                .justify_end()
                .gap(px(8.))
                .child(
                    button(
                        "offer-later",
                        "Не сейчас",
                        None,
                        Variant::Quiet,
                        Size::M,
                        accent,
                    )
                    .on_click(cx.listener(|offer, _, _, cx| offer.close(cx))),
                )
                .child(
                    button(
                        "offer-install",
                        format!(
                            "Подключить {offered} {}",
                            plural(offered, "плагин", "плагина", "плагинов")
                        ),
                        Some("puzzle"),
                        Variant::Primary,
                        Size::M,
                        accent,
                    )
                    .when(offered == 0 || self.installing, |b| b.opacity(0.5))
                    .on_click(cx.listener(move |offer, _, _, cx| {
                        if offered > 0 {
                            offer.install(cx)
                        }
                    })),
                )
                .into_any_element(),
        );
        let body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .children(parts)
            .into_any_element();
        let me = cx.entity().downgrade();
        div().children(modal(
            "repo-offer",
            self.open,
            "Подключить плагины",
            body,
            Rc::new(move |_, cx| {
                me.update(cx, |offer, cx| offer.close(cx)).ok();
            }),
            window,
            cx,
        ))
    }
}
