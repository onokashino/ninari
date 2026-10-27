//! The settings: the sections down the side, one of them beside them.

use std::sync::Arc;

use gpui::{
    AnyElement, Context, Div, Entity, EventEmitter, FontWeight, Hsla, IntoElement, Render, Window,
    div, prelude::*, px, rgb,
};
use serde_json::json;

use crate::assets::icon;
use crate::button::{Size, Variant, button};
use crate::core::Core;
use crate::data::{Load, fetch};
use crate::episode_grid::chip;
use crate::nav::PageEvent;
use crate::player_panels::switch;
use crate::rail::{self, brand_mark};
use crate::settings_data::DataPane;
use crate::settings_devices::DevicesPane;
use crate::settings_mal::MalPane;
use crate::settings_plugins::PluginsPane;
use crate::settings_shikimori::ShikimoriPane;
use crate::theme::{self, Accent};
use crate::updates::{self, CURRENT, Progress, UpdateInfo, check};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Sources,
    Integrations,
    Devices,
    Player,
    App,
    Data,
}

const SECTIONS: [(Section, &str, &str); 6] = [
    (Section::Sources, "Источники", "puzzle"),
    (Section::Integrations, "Интеграции", "user"),
    (Section::Devices, "Устройства", "devices"),
    (Section::Player, "Плеер", "sparkle"),
    (Section::App, "Приложение", "settings"),
    (Section::Data, "Данные", "data"),
];

const TIERS: [(&str, &str, &str); 4] = [
    ("off", "Выключен", "Смотреть в исходном качестве."),
    (
        "fast",
        "Fast",
        "Для большинства видеокарт, включая встроенные.",
    ),
    (
        "hq",
        "HQ",
        "Лучше картинка, нужна дискретная видеокарта уровня GTX 1660 и выше.",
    ),
    (
        "ultra",
        "Ultra",
        "Самые большие сети, для мощных видеокарт уровня RTX 3070 и выше.",
    ),
];
pub const TIER_SETTING: &str = "player.anime4k";

/// On/off settings that are on until the user turns them off.
const SWITCHES: [(&str, &str, &str); 3] = [
    (
        "app.auto_update",
        "Проверять обновления при запуске",
        "Если вышла новая версия, в углу окна появится предложение.",
    ),
    (
        "app.close_to_tray",
        "Сворачивать в трей при закрытии",
        "Окно прячется, а новые серии продолжают проверяться. Выйти можно из меню значка.",
    ),
    (
        "notify.enabled",
        "Уведомлять о новых сериях",
        "Раз в полчаса проверяю онгоинги из «Смотрю».",
    ),
];

pub struct SettingsPage {
    core: Arc<Core>,
    section: Section,
    shikimori: Option<Entity<ShikimoriPane>>,
    mal: Option<Entity<MalPane>>,
    plugins: Option<Entity<PluginsPane>>,
    devices: Option<Entity<DevicesPane>>,
    data: Option<Entity<DataPane>>,
    tier: String,
    switches: [bool; 3],
    /// The last update check: None before one, then the newer version or none.
    update: Option<Load<Option<UpdateInfo>>>,
}

impl EventEmitter<PageEvent> for SettingsPage {}

impl SettingsPage {
    pub fn new(core: Arc<Core>, _cx: &mut Context<Self>) -> Self {
        let tier = core
            .setting::<String>(TIER_SETTING)
            .unwrap_or_else(|| "fast".into());
        let switches = SWITCHES.map(|(key, ..)| core.setting::<bool>(key).unwrap_or(true));
        Self {
            core,
            section: Section::Sources,
            shikimori: None,
            mal: None,
            plugins: None,
            devices: None,
            data: None,
            tier,
            switches,
            update: None,
        }
    }

    pub fn open(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        cx.notify();
    }

    /// A section with a view of its own, made the first time it is opened; what it asks of the
    /// window passes through this page.
    fn pane<V: Render + EventEmitter<PageEvent>>(
        slot: &mut Option<Entity<V>>,
        core: &Arc<Core>,
        make: fn(Arc<Core>, &mut Context<V>) -> V,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if slot.is_none() {
            let core = core.clone();
            let pane = cx.new(|cx| make(core, cx));
            cx.subscribe(&pane, |_, _, event: &PageEvent, cx| cx.emit(event.clone()))
                .detach();
            *slot = Some(pane);
        }
        slot.clone().unwrap().into_any_element()
    }

    fn nav(&self, narrow: bool, accent: Hsla, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .when(!narrow, |d| d.flex_col().w(px(210.)).flex_none())
            .when(narrow, |d| d.flex_wrap())
            .gap(px(2.))
            .children(SECTIONS.map(|(section, label, glyph)| {
                let current = section == self.section;
                div()
                    .id(label)
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(12.))
                    .py(px(9.))
                    .rounded(theme::R_M)
                    .font_weight(FontWeight::BOLD)
                    .whitespace_nowrap()
                    .map(|d| {
                        if current {
                            d.bg(accent.opacity(0.18)).text_color(rgb(theme::TEXT))
                        } else {
                            d.text_color(rgb(theme::TEXT_2))
                                .hover(|s| s.bg(theme::glass()).text_color(rgb(theme::TEXT)))
                        }
                    })
                    .on_click(cx.listener(move |page, _, _, cx| page.open(section, cx)))
                    .child(icon(glyph, 20.).text_color(if current {
                        accent
                    } else {
                        rgb(theme::TEXT_2).into()
                    }))
                    .child(label)
            }))
    }

    fn check_update(&mut self, cx: &mut Context<Self>) {
        if matches!(self.update, Some(Load::Pending)) {
            return;
        }
        self.update = Some(Load::Pending);
        fetch(cx, check(&self.core), |page, found| {
            page.update = Some(found)
        });
        cx.notify();
    }

    fn player(&self, accent: Hsla, cx: &mut Context<Self>) -> Div {
        card("Плеер")
            .child(setting_text(
                "Апскейл",
                "Включается сам, когда видео меньше экрана. Метод, режим и сравнение с \
                 оригиналом настраиваются прямо в плеере кнопкой ✦; клавиша U включает и \
                 выключает.",
            ))
            .child(tier_picker(&self.tier, accent, cx, |page, tier, cx| {
                page.tier = tier.into();
                page.core.save_setting(TIER_SETTING, json!(tier));
                cx.notify();
            }))
    }

    fn app(&self, accent: Hsla, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let version = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(brand_mark(40.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child("ninari"),
                    )
                    .child(muted(match &self.update {
                        Some(Load::Ready(None)) => format!("Версия {CURRENT}, последняя"),
                        _ => format!("Версия {CURRENT}"),
                    })),
            );
        let checking = matches!(self.update, Some(Load::Pending));
        let found = match &self.update {
            Some(Load::Ready(Some(info))) => Some(info.clone()),
            _ => None,
        };
        let progress = updates::progress(cx);
        let busy = matches!(progress, Progress::Downloading(_) | Progress::Installing);
        let action = match &found {
            Some(info) => {
                let label = match &progress {
                    Progress::Downloading(share) => {
                        format!("Скачиваем {}%", (share * 100.).round())
                    }
                    Progress::Installing => "Устанавливаем".into(),
                    _ if updates::managed() => {
                        format!("{} придёт через пакетный менеджер", info.version)
                    }
                    _ if updates::can_install(info) => format!("Обновить до {}", info.version),
                    _ => format!("Скачать {}", info.version),
                };
                let info = info.clone();
                button(
                    "open-release",
                    label,
                    None,
                    Variant::Primary,
                    Size::M,
                    accent,
                )
                .when(busy, |b| b.opacity(0.6))
                .on_click(move |_, _, cx| updates::install(&info, cx))
            }
            None => button(
                "check-update",
                "Проверить обновления",
                Some("retry"),
                Variant::Glass,
                Size::M,
                accent,
            )
            .when(checking, |b| b.opacity(0.6))
            .on_click(cx.listener(|page, _, _, cx| page.check_update(cx))),
        };
        let mut out = card("Приложение")
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(24.))
                    .child(version)
                    .child(action),
            )
            .children(found.and_then(|i| i.notes).map(muted))
            .child(setting_row(
                setting_text(
                    "Что-то не так?",
                    "Откроется новая задача на GitHub: версия, система и конец лога уже вписаны, осталось описать, что случилось.",
                ),
                button(
                    "report",
                    "Сообщить о проблеме",
                    None,
                    Variant::Glass,
                    Size::M,
                    accent,
                )
                .on_click(|_, _, cx| cx.open_url(&crate::report::issue_url())),
            ))
            .children(match (&self.update, progress) {
                (_, Progress::Failed(e)) => Some(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(theme::DANGER))
                        .child(format!("Не удалось обновить: {e}")),
                ),
                (Some(Load::Failed(e)), _) => Some(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(theme::DANGER))
                        .child(e.clone()),
                ),
                _ => None,
            });
        for (i, (key, title, hint)) in SWITCHES.into_iter().enumerate() {
            let on = self.switches[i];
            out = out.child(setting_row(
                setting_text(title, hint),
                switch(("setting", i), on, accent, window, cx).on_click(cx.listener(
                    move |page, _, _, cx| {
                        page.switches[i] = !on;
                        page.core.save_setting(key, json!(!on));
                        cx.notify();
                    },
                )),
            ));
        }
        out
    }
}

/// The Anime4K tier with what each one asks of the video card; `pick` gets the chosen one.
pub fn tier_picker<V: 'static>(
    current: &str,
    accent: Hsla,
    cx: &mut Context<V>,
    pick: fn(&mut V, &'static str, &mut Context<V>),
) -> Div {
    let hint = TIERS.iter().find(|(v, ..)| *v == current).map(|(.., h)| *h);
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.))
                .children(TIERS.map(|(value, label, _)| {
                    chip(value, value == current, accent)
                        .on_click(cx.listener(move |view, _, _, cx| pick(view, value, cx)))
                        .child(label)
                })),
        )
        .children(hint.map(muted))
}

/// A section's card: a heading over its settings.
pub fn card(title: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .p(px(24.))
        .max_w(px(760.))
        .rounded(theme::R_L)
        .bg(theme::glass())
        .border_1()
        .border_color(theme::line())
        .child(
            div()
                .text_size(px(18.))
                .font_weight(FontWeight::EXTRA_BOLD)
                .child(title),
        )
}

pub fn muted(text: impl Into<gpui::SharedString>) -> Div {
    div()
        .text_size(px(13.))
        .text_color(rgb(theme::TEXT_3))
        .child(text.into())
}

/// A setting's name with its explanation under it.
pub fn setting_text(title: &'static str, hint: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .min_w(px(0.))
        .child(div().font_weight(FontWeight::BOLD).child(title))
        .child(muted(hint))
}

/// The explanation on the left, the control on the right.
pub fn setting_row(text: Div, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(24.))
        .py(px(8.))
        .child(text.flex_1())
        .child(control)
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        let narrow = f32::from(window.viewport_size().width - px(rail::WIDTH)) < 960.;
        let pane: AnyElement = match self.section {
            Section::Player => self.player(accent, cx).into_any_element(),
            Section::App => self.app(accent, window, cx).into_any_element(),
            Section::Sources => Self::pane(&mut self.plugins, &self.core, PluginsPane::new, cx),
            Section::Devices => Self::pane(&mut self.devices, &self.core, DevicesPane::new, cx),
            Section::Data => Self::pane(&mut self.data, &self.core, DataPane::new, cx),
            Section::Integrations => div()
                .flex()
                .flex_col()
                .gap(px(24.))
                .child(Self::pane(
                    &mut self.shikimori,
                    &self.core,
                    ShikimoriPane::new,
                    cx,
                ))
                .child(Self::pane(&mut self.mal, &self.core, MalPane::new, cx))
                .into_any_element(),
        };
        div()
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
                    .child("Настройки"),
            )
            .child(
                div()
                    .flex()
                    .when(narrow, |d| d.flex_col())
                    .items_start()
                    .gap(px(if narrow { 16. } else { 32. }))
                    .child(self.nav(narrow, accent, cx))
                    .child(div().flex_1().min_w(px(0.)).max_w(px(760.)).child(pane)),
            )
    }
}
