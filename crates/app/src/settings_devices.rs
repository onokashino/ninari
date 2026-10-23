//! The devices section: local network sync on or off, this device's pairing code, devices nearby to
//! pair with, and paired ones.

use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, Context, Div, Entity, EventEmitter, FontWeight, IntoElement, Render, Window, div,
    prelude::*, px, rgb,
};
use ninari_core::db;
use ninari_core::devices::lan::Nearby;
use ninari_core::devices::{ENABLED_SETTING, PairingCode, Peer};
use ninari_core::error::AppError;
use serde_json::json;

use crate::button::{Size, Variant, button};
use crate::clock::now;
use crate::core::Core;
use crate::format::plural;
use crate::nav::PageEvent;
use crate::player_panels::switch;
use crate::settings_page::{card, muted, setting_row, setting_text};
use crate::settings_plugins::time_ago;
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, Accent};

/// "482915" as "482 915": easier to read out and type.
fn spaced(code: &str) -> String {
    match code.char_indices().nth(3) {
        Some((i, _)) => format!("{} {}", &code[..i], &code[i..]),
        None => code.to_string(),
    }
}

fn digits(text: &str) -> String {
    text.chars().filter(char::is_ascii_digit).collect()
}

pub struct DevicesPane {
    core: Arc<Core>,
    enabled: bool,
    pairing: Option<PairingCode>,
    peers: Vec<(Peer, bool)>,
    nearby: Vec<Nearby>,
    /// The nearby device whose code is being typed.
    typing: Option<String>,
    code: Entity<TextInput>,
    pairing_busy: bool,
    synced: Option<(usize, usize)>,
    syncing: bool,
    error: Option<String>,
}

impl EventEmitter<PageEvent> for DevicesPane {}

impl DevicesPane {
    pub fn new(core: Arc<Core>, cx: &mut Context<Self>) -> Self {
        let code = cx.new(|cx| {
            let mut input = TextInput::new("Код с другого устройства", cx);
            input.compact = true;
            input.search = false;
            input
        });
        cx.subscribe(&code, |pane, _, event: &InputEvent, cx| match event {
            InputEvent::Submit => pane.pair(cx),
            _ => cx.notify(),
        })
        .detach();
        // The countdown and the devices coming and going on the network.
        cx.spawn(async move |pane, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if pane.update(cx, |pane, cx| pane.refresh(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let mut pane = Self {
            core,
            enabled: false,
            pairing: None,
            peers: Vec::new(),
            nearby: Vec::new(),
            typing: None,
            code,
            pairing_busy: false,
            synced: None,
            syncing: false,
            error: None,
        };
        pane.refresh(cx);
        pane
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let c = &self.core;
        self.enabled = c.lan.is_running();
        self.pairing = c.devices.pairing();
        let peers = c.devices.peers().unwrap_or_default();
        let nearby = c.lan.nearby();
        self.peers = peers
            .into_iter()
            .map(|p| {
                let online = nearby.iter().any(|n| n.id == p.id);
                (p, online)
            })
            .collect();
        self.nearby = nearby
            .into_iter()
            .filter(|n| !self.peers.iter().any(|(p, _)| p.id == n.id))
            .collect();
        cx.notify();
    }

    /// Runs `job` on the core, then shows the devices as they are.
    fn act(
        &mut self,
        job: impl Future<Output = anyhow::Result<()>> + 'static,
        cx: &mut Context<Self>,
    ) {
        self.error = None;
        cx.spawn(async move |pane, cx| {
            let got = job.await;
            pane.update(cx, |pane, cx| {
                if let Err(e) = got {
                    pane.error = Some(format!("{e:#}"));
                }
                pane.refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    fn set_enabled(&mut self, on: bool, cx: &mut Context<Self>) {
        let job = self.core.run(move |c| async move {
            db::set_setting(&c.db.lock().unwrap(), ENABLED_SETTING, &json!(on))?;
            if on {
                c.lan.start().await
            } else {
                c.lan.stop();
                Ok(())
            }
        });
        self.act(job, cx);
    }

    fn show_code(&mut self, cx: &mut Context<Self>) {
        if !self.core.lan.is_running() {
            self.error = Some("Сначала включи синхронизацию в локальной сети".into());
        } else {
            self.core.devices.start_pairing();
        }
        self.refresh(cx);
    }

    fn pair(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.typing.clone() else {
            return;
        };
        let code = digits(self.code.read(cx).text());
        if code.len() != 6 || self.pairing_busy {
            return;
        }
        self.pairing_busy = true;
        self.error = None;
        let job = self.core.run(move |c| async move {
            let addr = c
                .lan
                .addr_of(&id)
                .ok_or_else(|| AppError::new("Devices", "Устройство пропало из сети"))?;
            let peer = c.devices.pair(addr, &code).await?;
            let _ = c.devices.sync_with(&peer.id, addr).await;
            Ok(())
        });
        cx.spawn(async move |pane, cx| {
            let got = job.await;
            pane.update(cx, |pane, cx| {
                pane.pairing_busy = false;
                match got {
                    Ok(()) => {
                        pane.typing = None;
                        pane.code.update(cx, |input, cx| input.set_text("", cx));
                        cx.emit(PageEvent::Changed);
                    }
                    Err(e) => pane.error = Some(format!("{e:#}")),
                }
                pane.refresh(cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn sync_now(&mut self, cx: &mut Context<Self>) {
        if self.syncing {
            return;
        }
        self.syncing = true;
        let job = self
            .core
            .run(|c| async move { Ok::<_, AppError>(c.lan.sync_all().await) });
        cx.spawn(async move |pane, cx| {
            let got = job.await;
            pane.update(cx, |pane, cx| {
                pane.syncing = false;
                match got {
                    Ok(counts) => {
                        pane.synced = Some(counts);
                        cx.emit(PageEvent::Changed);
                    }
                    Err(e) => pane.error = Some(format!("{e:#}")),
                }
                pane.refresh(cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn code_block(&self, pairing: &PairingCode, accent: gpui::Hsla, cx: &mut Context<Self>) -> Div {
        let left = pairing.expires_at - now();
        if left <= 0 {
            return div()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(8.))
                .child(muted("Код истёк."))
                .child(
                    button(
                        "new-code",
                        "Показать новый код",
                        None,
                        Variant::Glass,
                        Size::M,
                        accent,
                    )
                    .on_click(cx.listener(|pane, _, _, cx| pane.show_code(cx))),
                );
        }
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(40.))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .text_color(accent)
                    .child(spaced(&pairing.code)),
            )
            .child(muted(format!(
                "Введи этот код на другом устройстве. Он сработает один раз, осталось {}:{:02}.",
                left / 60,
                left % 60
            )))
            .child(
                button(
                    "hide-code",
                    "Скрыть код",
                    None,
                    Variant::Quiet,
                    Size::M,
                    accent,
                )
                .on_click(cx.listener(|pane, _, _, cx| {
                    pane.core.devices.stop_pairing();
                    pane.refresh(cx);
                })),
            )
    }

    fn nearby_row(
        &self,
        n: &Nearby,
        i: usize,
        accent: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let typing = self.typing.as_deref() == Some(n.id.as_str());
        let id = n.id.clone();
        let ready = digits(self.code.read(cx).text()).len() == 6;
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
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::BOLD)
                            .child(n.name.clone()),
                    )
                    .when(!typing, |d| {
                        d.child(
                            button(
                                ("enter-code", i),
                                "Ввести код",
                                None,
                                Variant::Glass,
                                Size::M,
                                accent,
                            )
                            .on_click(cx.listener(
                                move |pane, _, window, cx| {
                                    pane.typing = Some(id.clone());
                                    pane.code.update(cx, |input, cx| {
                                        input.set_text("", cx);
                                        input.focus(window, cx);
                                    });
                                    cx.notify();
                                },
                            )),
                        )
                    }),
            )
            .when(typing, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(div().flex_1().max_w(px(280.)).child(self.code.clone()))
                        .child(
                            button("pair", "Связать", None, Variant::Primary, Size::M, accent)
                                .when(!ready || self.pairing_busy, |b| b.opacity(0.5))
                                .on_click(cx.listener(|pane, _, _, cx| pane.pair(cx))),
                        ),
                )
            })
            .into_any_element()
    }
}

impl Render for DevicesPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        let on = self.enabled;
        let mut out = card("Устройства").child(setting_row(
            setting_text(
                "Синхронизация в локальной сети",
                "Библиотека, серии, место в серии и озвучки одинаковы на всех связанных устройствах в этой сети.",
            ),
            switch("lan-on", on, accent, window, cx)
                .on_click(cx.listener(move |pane, _, _, cx| pane.set_enabled(!on, cx))),
        ));
        if on {
            let t = now();
            let online = self.peers.iter().filter(|(_, o)| *o).count();
            out = out
                .child(
                    div()
                        .flex()
                        .gap(px(4.))
                        .child(muted("Это устройство:"))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::BOLD)
                                .child(self.core.devices.name.clone()),
                        ),
                )
                .when(!self.peers.is_empty(), |d| {
                    d.child(div().flex().flex_col().gap(px(8.)).children(
                        self.peers.iter().enumerate().map(|(i, (p, online))| {
                            let id = p.id.clone();
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(12.))
                                .px(px(16.))
                                .py(px(12.))
                                .rounded(theme::R_M)
                                .bg(theme::glass())
                                .child(div().font_weight(FontWeight::BOLD).child(p.name.clone()))
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(rgb(if *online {
                                            theme::OK
                                        } else {
                                            theme::TEXT_3
                                        }))
                                        .child(if *online {
                                            "в сети"
                                        } else {
                                            "не в сети"
                                        }),
                                )
                                .child(muted(match p.last_sync {
                                    Some(at) => format!("синхронизировано {}", time_ago(at, t)),
                                    None => "ещё не синхронизировалось".into(),
                                }))
                                .child(
                                    div().ml_auto().child(
                                        button(
                                            ("unpair", i),
                                            "Отвязать",
                                            None,
                                            Variant::Quiet,
                                            Size::M,
                                            accent,
                                        )
                                        .on_click(
                                            cx.listener(move |pane, _, _, cx| {
                                                if let Err(e) = pane.core.devices.unpair(&id) {
                                                    pane.error = Some(e.message);
                                                }
                                                pane.refresh(cx);
                                            }),
                                        ),
                                    ),
                                )
                        }),
                    ))
                })
                .when(online > 0, |d| {
                    let result = self.synced.map(|(titles, episodes)| {
                        if titles + episodes == 0 {
                            "Всё уже совпадает.".to_string()
                        } else {
                            let (t, e) = (titles as i64, episodes as i64);
                            format!(
                                "Пришло {t} {} и {e} {}.",
                                plural(t, "тайтл", "тайтла", "тайтлов"),
                                plural(e, "серия", "серии", "серий")
                            )
                        }
                    });
                    d.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(12.))
                            .child(
                                button(
                                    "sync-now",
                                    "Синхронизировать сейчас",
                                    None,
                                    Variant::Glass,
                                    Size::M,
                                    accent,
                                )
                                .when(self.syncing, |b| b.opacity(0.5))
                                .on_click(cx.listener(|pane, _, _, cx| pane.sync_now(cx))),
                            )
                            .children(result.map(muted)),
                    )
                })
                .child(
                    div()
                        .mt(px(12.))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .child("Связать устройство"),
                );
            out = match &self.pairing {
                Some(p) => out.child(self.code_block(p, accent, cx)),
                None => out.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(12.))
                        .child(
                            button(
                                "show-code",
                                "Показать код",
                                None,
                                Variant::Glass,
                                Size::M,
                                accent,
                            )
                            .on_click(cx.listener(|pane, _, _, cx| pane.show_code(cx))),
                        )
                        .child(muted(
                            "и введи его на другом устройстве, где ninari тоже открыт в этой сети",
                        )),
                ),
            };
            out = if self.nearby.is_empty() {
                out.child(muted(
                    "Других устройств с включённой синхронизацией в этой сети пока не видно.",
                ))
            } else {
                let rows: Vec<AnyElement> = self
                    .nearby
                    .iter()
                    .enumerate()
                    .map(|(i, n)| self.nearby_row(n, i, accent, cx))
                    .collect();
                out.child(div().flex().flex_col().gap(px(8.)).children(rows))
            };
        }
        out.children(self.error.clone().map(|e| {
            div()
                .text_size(px(13.))
                .text_color(rgb(theme::DANGER))
                .child(e)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_read_in_two_halves_and_keep_only_digits() {
        assert_eq!(spaced("482915"), "482 915");
        assert_eq!(digits("482 915"), "482915");
        assert_eq!(digits("48-29a15"), "482915");
    }
}
