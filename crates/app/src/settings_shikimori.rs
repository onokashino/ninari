//! The Shikimori section: signing in, after which statuses, episodes and scores go to the list
//! there too, and importing a public list.

use std::sync::Arc;

use gpui::{Context, Entity, EventEmitter, IntoElement, Render, Window, div, prelude::*, px, rgb};
use ninari_core::account::{self, Profile};
use ninari_core::sync::{ImportReport, import_list};

use crate::button::{Size, Variant, button};
use crate::core::Core;
use crate::data::Load;
use crate::format::plural;
use crate::nav::PageEvent;
use crate::settings_page::{card, muted, setting_row, setting_text};
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, Accent};

/// A Shikimori nickname in, the titles of that public list added to the library; in the settings
/// and in the first start's steps.
pub struct ImportForm {
    core: Arc<Core>,
    nickname: Entity<TextInput>,
    import: Option<Load<ImportReport>>,
}

impl EventEmitter<PageEvent> for ImportForm {}

impl ImportForm {
    pub fn new(core: Arc<Core>, cx: &mut Context<Self>) -> Self {
        let known = core.account.profile().map(|p| p.nickname);
        let nickname = cx.new(|cx| {
            let mut input = TextInput::new("Никнейм на Shikimori", cx);
            input.compact = true;
            input.search = false;
            if let Some(n) = known {
                input.set_text(n, cx);
            }
            input
        });
        cx.subscribe(&nickname, |form, _, event: &InputEvent, cx| match event {
            InputEvent::Submit => form.import(cx),
            _ => cx.notify(),
        })
        .detach();
        Self {
            core,
            nickname,
            import: None,
        }
    }

    pub fn set_nickname(&mut self, nickname: String, cx: &mut Context<Self>) {
        self.nickname
            .update(cx, |input, cx| input.set_text(nickname, cx));
    }

    fn import(&mut self, cx: &mut Context<Self>) {
        let nickname = self.nickname.read(cx).text().trim().to_string();
        if nickname.is_empty() || matches!(self.import, Some(Load::Pending)) {
            return;
        }
        self.import = Some(Load::Pending);
        let job = self
            .core
            .run(move |c| async move { import_list(&c.catalog, &c.library, &nickname).await });
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

impl Render for ImportForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        let busy = matches!(self.import, Some(Load::Pending));
        let empty = self.nickname.read(cx).text().trim().is_empty();
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(div().flex_1().max_w(px(360.)).child(self.nickname.clone()))
                    .child(
                        button(
                            "import",
                            if busy {
                                "Импортирую…"
                            } else {
                                "Импортировать"
                            },
                            None,
                            Variant::Glass,
                            Size::M,
                            accent,
                        )
                        .when(busy || empty, |b| b.opacity(0.5))
                        .on_click(cx.listener(|form, _, _, cx| form.import(cx))),
                    ),
            )
            .children(match &self.import {
                Some(Load::Ready(r)) => Some(muted(report_line(r)).into_any_element()),
                Some(Load::Failed(e)) => Some(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(theme::DANGER))
                        .child(e.clone())
                        .into_any_element(),
                ),
                _ => None,
            })
    }
}

pub struct ShikimoriPane {
    core: Arc<Core>,
    profile: Option<Profile>,
    /// Why the last change did not reach Shikimori.
    sync_error: Option<String>,
    signing_in: bool,
    sign_in_error: Option<String>,
    import: Entity<ImportForm>,
}

impl EventEmitter<PageEvent> for ShikimoriPane {}

impl ShikimoriPane {
    pub fn new(core: Arc<Core>, cx: &mut Context<Self>) -> Self {
        let import = {
            let core = core.clone();
            cx.new(|cx| ImportForm::new(core, cx))
        };
        cx.subscribe(&import, |_, _, event: &PageEvent, cx| {
            cx.emit(event.clone())
        })
        .detach();
        Self {
            sync_error: core.account.last_error(),
            profile: core.account.profile(),
            core,
            signing_in: false,
            sign_in_error: None,
            import,
        }
    }

    /// Opens Shikimori in the browser and waits there for the user to allow access.
    fn sign_in(&mut self, cx: &mut Context<Self>) {
        self.signing_in = true;
        self.sign_in_error = None;
        let (url_tx, url_rx) = tokio::sync::oneshot::channel::<String>();
        let job = self.core.run(move |c| async move {
            c.account
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
                        let nickname = profile.nickname.clone();
                        pane.import
                            .update(cx, |form, cx| form.set_nickname(nickname, cx));
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
        if let Err(e) = self.core.account.sign_out() {
            self.sign_in_error = Some(e.message);
        }
        self.profile = None;
        self.sync_error = None;
        cx.notify();
    }
}

pub fn report_line(r: &ImportReport) -> String {
    if r.total == 0 {
        return "Список пуст.".into();
    }
    let added = r.added as i64;
    let mut line = format!(
        "Добавлено {added} {}",
        plural(added, "тайтл", "тайтла", "тайтлов")
    );
    if r.skipped > 0 {
        let skipped = r.skipped as i64;
        line += &format!(
            ", {skipped} уже {} в библиотеке.",
            plural(skipped, "был", "были", "были")
        );
    } else {
        line.push('.');
    }
    line
}

impl Render for ShikimoriPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        let error = |text: String| {
            div()
                .text_size(px(13.))
                .text_color(rgb(theme::DANGER))
                .child(text)
        };
        let mut out = card("Shikimori");
        if !account::configured() {
            out = out.child(muted(
                "Вход через Shikimori в этой сборке выключен: ей не дали ключи приложения \
                 (SHIKIMORI_CLIENT_ID и SHIKIMORI_CLIENT_SECRET).",
            ));
        } else if let Some(p) = &self.profile {
            out = out
                .child(setting_row(
                    setting_text(
                        "Вход выполнен",
                        "Список синхронизируется в обе стороны: изменения отсюда уходят на Shikimori сразу, оттуда приходят раз в полчаса.",
                    )
                    .child(div().font_weight(gpui::FontWeight::BOLD).child(p.nickname.clone())),
                    button("sign-out", "Выйти", None, Variant::Glass, Size::M, accent)
                        .on_click(cx.listener(|pane, _, _, cx| pane.sign_out(cx))),
                ))
                .children(self.sync_error.clone().map(|e| {
                    error(format!("Последнее изменение не дошло до Shikimori: {e}"))
                }));
        } else {
            let control = if self.signing_in {
                button(
                    "sign-cancel",
                    "Отмена",
                    None,
                    Variant::Glass,
                    Size::M,
                    accent,
                )
                .on_click(cx.listener(|pane, _, _, cx| {
                    pane.core.account.cancel();
                    cx.notify();
                }))
            } else {
                button(
                    "sign-in",
                    "Войти через Shikimori",
                    None,
                    Variant::Primary,
                    Size::M,
                    accent,
                )
                .on_click(cx.listener(|pane, _, _, cx| pane.sign_in(cx)))
            };
            out = out
                .child(setting_row(
                    setting_text(
                        "Аккаунт Shikimori",
                        if self.signing_in {
                            "Подтверди вход в открывшемся браузере."
                        } else {
                            "Откроется браузер. После входа список здесь и на Shikimori будет одним: статусы, серии и оценки в обе стороны."
                        },
                    ),
                    control,
                ))
                .children(self.sign_in_error.clone().map(error));
        }

        out.child(setting_text(
            "Импорт списка",
            "Тайтлы открытого списка со статусами и сериями. То, что уже есть в библиотеке, не трогается.",
        ))
        .child(self.import.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_import_report_reads_as_a_sentence() {
        let r = |total, added, skipped| ImportReport {
            total,
            added,
            skipped,
        };
        assert_eq!(report_line(&r(0, 0, 0)), "Список пуст.");
        assert_eq!(report_line(&r(5, 5, 0)), "Добавлено 5 тайтлов.");
        assert_eq!(
            report_line(&r(3, 1, 2)),
            "Добавлено 1 тайтл, 2 уже были в библиотеке."
        );
    }
}
