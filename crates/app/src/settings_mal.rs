//! The MyAnimeList card of the integrations: signing in, after which statuses, episodes and
//! scores go to the list there too, and taking that list into the library.

use std::sync::Arc;

use gpui::{Context, EventEmitter, IntoElement, Render, Window, div, prelude::*, px, rgb};
use ninari_core::account::Profile;
use ninari_core::mal;
use ninari_core::sync::ImportReport;

use crate::button::{Size, Variant, button};
use crate::core::Core;
use crate::data::Load;
use crate::nav::PageEvent;
use crate::settings_page::{card, muted, setting_row, setting_text};
use crate::settings_shikimori::report_line;
use crate::theme::{self, Accent};

pub struct MalPane {
    core: Arc<Core>,
    profile: Option<Profile>,
    /// Why the last change did not reach MyAnimeList.
    sync_error: Option<String>,
    signing_in: bool,
    sign_in_error: Option<String>,
    import: Option<Load<ImportReport>>,
}

impl EventEmitter<PageEvent> for MalPane {}

impl MalPane {
    pub fn new(core: Arc<Core>, _cx: &mut Context<Self>) -> Self {
        Self {
            sync_error: core.account.mal.last_error(),
            profile: core.account.mal.profile(),
            core,
            signing_in: false,
            sign_in_error: None,
            import: None,
        }
    }

    /// Opens MyAnimeList in the browser and waits there for the user to allow access.
    fn sign_in(&mut self, cx: &mut Context<Self>) {
        self.signing_in = true;
        self.sign_in_error = None;
        let (url_tx, url_rx) = tokio::sync::oneshot::channel::<String>();
        let job = self.core.run(move |c| async move {
            c.account
                .mal
                .sign_in(move |url| {
                    let _ = url_tx.send(url.to_string());
                    Ok(())
                })
                .await
        });
        cx.spawn(async move |pane, cx| {
            if let Ok(url) = url_rx.await {
                cx.update(|cx| cx.open_url(&url));
            }
            let got = job.await;
            pane.update(cx, |pane, cx| {
                pane.signing_in = false;
                match got {
                    Ok(profile) => {
                        pane.profile = Some(profile);
                        pane.sync_error = None;
                    }
                    Err(e) => pane.sign_in_error = Some(format!("{e:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn sign_out(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = self.core.account.mal.sign_out() {
            self.sign_in_error = Some(e.message);
        }
        self.profile = None;
        self.sync_error = None;
        self.import = None;
        cx.notify();
    }

    fn import(&mut self, cx: &mut Context<Self>) {
        if matches!(self.import, Some(Load::Pending)) {
            return;
        }
        self.import = Some(Load::Pending);
        let job = self
            .core
            .run(move |c| async move { c.account.mal.import(&c.catalog).await });
        cx.spawn(async move |pane, cx| {
            let got = job.await;
            pane.update(cx, |pane, cx| {
                if got.is_ok() {
                    cx.emit(PageEvent::Changed);
                }
                pane.import = Some(got.into());
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

impl Render for MalPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        let error = |text: String| {
            div()
                .text_size(px(13.))
                .text_color(rgb(theme::DANGER))
                .child(text)
        };
        let out = card("MyAnimeList");
        if !mal::configured() {
            return out.child(muted(
                "Вход через MyAnimeList в этой сборке выключен: ей не дали ключ приложения \
                 (MAL_CLIENT_ID).",
            ));
        }
        let Some(p) = &self.profile else {
            let control = if self.signing_in {
                button(
                    "mal-cancel",
                    "Отмена",
                    None,
                    Variant::Glass,
                    Size::M,
                    accent,
                )
                .on_click(cx.listener(|pane, _, _, cx| {
                    pane.core.account.mal.cancel();
                    cx.notify();
                }))
            } else {
                button(
                    "mal-sign-in",
                    "Войти через MyAnimeList",
                    None,
                    Variant::Primary,
                    Size::M,
                    accent,
                )
                .on_click(cx.listener(|pane, _, _, cx| pane.sign_in(cx)))
            };
            return out
                .child(setting_row(
                    setting_text(
                        "Аккаунт MyAnimeList",
                        if self.signing_in {
                            "Подтверди вход в открывшемся браузере."
                        } else {
                            "Откроется браузер. После входа статусы, серии и оценки будут попадать и в список на MyAnimeList, а его можно будет перенести сюда."
                        },
                    ),
                    control,
                ))
                .children(self.sign_in_error.clone().map(error));
        };
        let busy = matches!(self.import, Some(Load::Pending));
        out.child(setting_row(
            setting_text(
                "Вход выполнен",
                "Статусы, просмотренные серии и оценки отсюда уходят в твой список на MyAnimeList.",
            )
            .child(div().font_weight(gpui::FontWeight::BOLD).child(p.nickname.clone())),
            button("mal-sign-out", "Выйти", None, Variant::Glass, Size::M, accent)
                .on_click(cx.listener(|pane, _, _, cx| pane.sign_out(cx))),
        ))
        .children(
            self.sync_error
                .clone()
                .map(|e| error(format!("Последнее изменение не дошло до MyAnimeList: {e}"))),
        )
        .child(setting_row(
            setting_text(
                "Импорт списка",
                "Тайтлы твоего списка на MyAnimeList со статусами и сериями. То, что уже есть в библиотеке, не трогается.",
            ),
            button(
                "mal-import",
                if busy { "Импортирую…" } else { "Импортировать" },
                None,
                Variant::Glass,
                Size::M,
                accent,
            )
            .when(busy, |b| b.opacity(0.5))
            .on_click(cx.listener(|pane, _, _, cx| pane.import(cx))),
        ))
        .children(match &self.import {
            Some(Load::Ready(r)) => Some(muted(report_line(r)).into_any_element()),
            Some(Load::Failed(e)) => Some(error(e.clone()).into_any_element()),
            _ => None,
        })
    }
}
