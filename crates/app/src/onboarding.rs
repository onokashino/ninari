//! The first start: where episodes come from, the user's Shikimori list and the upscaler, over the
//! dimmed app until done or skipped.

use std::sync::Arc;

use gpui::{
    AnyElement, BoxShadow, Context, Div, Entity, EventEmitter, FontWeight, IntoElement, Render,
    Window, div, point, prelude::*, px, relative, rgb, rgba,
};
use serde_json::json;

use crate::assets::icon;
use crate::button::{Size, Variant, button};
use crate::core::Core;
use crate::data::{Load, fetch};
use crate::nav::PageEvent;
use crate::rail::brand_mark;
use crate::settings_page::{TIER_SETTING, muted, tier_picker};
use crate::settings_plugins::{Registry, install_from_registry, load_registry};
use crate::settings_shikimori::ImportForm;
use crate::theme::{self, Accent};

pub const ONBOARDED_SETTING: &str = "app.onboarded";
const STEPS: usize = 4;

/// The user finished or skipped the steps.
pub struct Done;

pub struct Onboarding {
    core: Arc<Core>,
    step: usize,
    registry: Load<Registry>,
    installing: bool,
    install_error: Option<String>,
    import: Entity<ImportForm>,
    tier: String,
}

impl EventEmitter<Done> for Onboarding {}
impl EventEmitter<PageEvent> for Onboarding {}

impl Onboarding {
    pub fn new(core: Arc<Core>, cx: &mut Context<Self>) -> Self {
        let import = {
            let core = core.clone();
            cx.new(|cx| ImportForm::new(core, cx))
        };
        cx.subscribe(&import, |_, _, event: &PageEvent, cx| {
            cx.emit(event.clone())
        })
        .detach();
        fetch(cx, load_registry(&core), |view, registry| {
            view.registry = registry
        });
        Self {
            tier: core
                .setting::<String>(TIER_SETTING)
                .unwrap_or_else(|| "fast".into()),
            core,
            step: 0,
            registry: Load::Pending,
            installing: false,
            install_error: None,
            import,
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.core.save_setting(ONBOARDED_SETTING, json!(true));
        cx.emit(Done);
    }

    fn go(&mut self, step: usize, cx: &mut Context<Self>) {
        self.step = step;
        cx.notify();
    }

    fn install_all(&mut self, cx: &mut Context<Self>) {
        if self.installing {
            return;
        }
        self.installing = true;
        self.install_error = None;
        let job = install_from_registry(&self.core, None);
        cx.spawn(async move |view, cx| {
            let got = job.await;
            view.update(cx, |view, cx| {
                view.installing = false;
                if let Err(e) = got {
                    view.install_error = Some(format!("{e:#}"));
                }
                fetch(cx, load_registry(&view.core), |view, registry| {
                    view.registry = registry
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn welcome(&self, accent: gpui::Hsla, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(16.))
            .child(brand_mark(84.))
            .child(
                div()
                    .mt(px(8.))
                    .text_size(px(34.))
                    .line_height(px(40.))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child("Добро пожаловать в ninari"),
            )
            .child(muted(
                "Каталог Shikimori, серии из подключённых источников и плеер с апскейлом в одном окне. \
                 Три коротких шага, и можно смотреть.",
            ))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        button("start", "Настроить", None, Variant::Primary, Size::M, accent)
                            .on_click(cx.listener(|view, _, _, cx| view.go(1, cx))),
                    )
                    .child(
                        button("skip-start", "Пропустить", None, Variant::Quiet, Size::M, accent)
                            .on_click(cx.listener(|view, _, _, cx| view.close(cx))),
                    ),
            )
    }

    fn sources(&self, accent: gpui::Hsla, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = vec![
            heading("Источники серий"),
            muted(
                "Плагины находят серии на сайтах с озвучками. Каждый работает в песочнице и ходит \
                 только на свои сайты.",
            )
            .into_any_element(),
        ];
        match &self.registry {
            Load::Pending => out.push(
                div()
                    .h(px(96.))
                    .rounded(theme::R_M)
                    .bg(theme::glass())
                    .into_any_element(),
            ),
            Load::Failed(e) => out.push(
                error(format!(
                    "Реестр плагинов недоступен: {e}. Подключить их можно позже в настройках."
                ))
                .into_any_element(),
            ),
            Load::Ready(r) => {
                out.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .my(px(8.))
                        .children(r.entries.iter().map(|e| {
                            let state = if e.installed_version.is_some() {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(theme::OK))
                                    .child(icon("check", 16.).text_color(rgb(theme::OK)))
                                    .child("подключён")
                            } else {
                                muted("не подключён")
                            };
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(12.))
                                .px(px(16.))
                                .py(px(12.))
                                .rounded(theme::R_M)
                                .bg(theme::glass())
                                .child(div().font_weight(FontWeight::BOLD).child(e.name.clone()))
                                .child(state)
                        }))
                        .into_any_element(),
                );
                if r.entries.iter().any(|e| e.installed_version.is_none()) {
                    out.push(
                        div()
                            .child(
                                button(
                                    "install-all",
                                    "Подключить все",
                                    Some("puzzle"),
                                    Variant::Glass,
                                    Size::M,
                                    accent,
                                )
                                .when(self.installing, |b| b.opacity(0.6))
                                .on_click(cx.listener(|view, _, _, cx| view.install_all(cx))),
                            )
                            .into_any_element(),
                    );
                }
            }
        }
        out.extend(
            self.install_error
                .clone()
                .map(|e| error(e).into_any_element()),
        );
        out
    }
}

fn heading(text: &'static str) -> AnyElement {
    div()
        .text_size(px(24.))
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(text)
        .into_any_element()
}

fn error(text: String) -> Div {
    div()
        .text_size(px(13.))
        .text_color(rgb(theme::DANGER))
        .child(text)
}

impl Render for Onboarding {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        let step = self.step;
        let body: AnyElement = if step == 0 {
            self.welcome(accent, cx).into_any_element()
        } else {
            let mut parts = vec![
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(accent)
                    .child(format!("Шаг {} из {STEPS}", step + 1))
                    .into_any_element(),
            ];
            match step {
                1 => parts.extend(self.sources(accent, cx)),
                2 => parts.extend([
                    heading("Список с Shikimori"),
                    muted(
                        "Если ведёшь список на Shikimori, перенесу тайтлы со статусами и \
                         просмотренными сериями. Список должен быть открытым.",
                    )
                    .into_any_element(),
                    self.import.clone().into_any_element(),
                ]),
                _ => parts.extend([
                    heading("Апскейл Anime4K"),
                    muted(
                        "Дорисовывает чёткость, когда видео меньше экрана. Потом его можно \
                         поменять в настройках или клавишей U в плеере.",
                    )
                    .into_any_element(),
                    tier_picker(&self.tier, accent, cx, |view, tier, cx| {
                        view.tier = tier.into();
                        view.core.save_setting(TIER_SETTING, json!(tier));
                        cx.notify();
                    })
                    .into_any_element(),
                ]),
            }
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .min_h(px(210.))
                .children(parts)
                .into_any_element()
        };
        let last = step == STEPS - 1;
        let footer = (step > 0).then(|| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .child(
                    button("skip", "Пропустить", None, Variant::Quiet, Size::M, accent)
                        .on_click(cx.listener(|view, _, _, cx| view.close(cx))),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            button("back", "Назад", None, Variant::Glass, Size::M, accent)
                                .on_click(cx.listener(move |view, _, _, cx| view.go(step - 1, cx))),
                        )
                        .child(
                            button(
                                "next",
                                if last { "Готово" } else { "Дальше" },
                                None,
                                Variant::Primary,
                                Size::M,
                                accent,
                            )
                            .on_click(cx.listener(
                                move |view, _, _, cx| {
                                    if last {
                                        view.close(cx)
                                    } else {
                                        view.go(step + 1, cx)
                                    }
                                },
                            )),
                        ),
                )
        });
        let view = window.viewport_size();
        // Anchored at the top: steps differ in height, and a centred panel would jump both ways.
        div()
            .id("onboarding")
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            .pt(view.height * 0.14)
            .px(px(32.))
            .bg(rgba(0x08080cc7))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(32.))
                    .w(relative(1.))
                    .max_w(px(560.))
                    .h_auto()
                    .self_start()
                    .p(px(32.))
                    .rounded(theme::R_L)
                    .bg(rgba(0x12121af7))
                    .border_1()
                    .border_color(rgba(0xffffff14))
                    .shadow(vec![BoxShadow {
                        color: rgba(0x0000008c).into(),
                        offset: point(px(0.), px(40.)),
                        blur_radius: px(100.),
                        spread_radius: px(0.),
                        inset: false,
                    }])
                    .child(body)
                    .children(footer),
            )
    }
}
