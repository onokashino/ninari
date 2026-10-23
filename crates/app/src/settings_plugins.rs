//! The sources section: plugins from the built-in registry, each with how it did this week,
//! installed with one click, and third-party repositories shown with the sites their plugins talk
//! to before they go in.

use std::f32::consts::PI;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, Div, Entity, EventEmitter, FontWeight, Hsla, IntoElement, Render,
    Transformation, Window, div, prelude::*, px, radians, rgb, rgba,
};
use ninari_core::db;
use ninari_core::extensions::registry::{REGISTRY_SETTING, RegistryEntry, merge, registry_url};
use ninari_core::extensions::types::Manifest;
use ninari_core::sources::SourceHealth;
use ninari_core::sources::dubs::DubTable;

use crate::assets::icon;
use crate::button::{Size, Variant, button};
use crate::clock::now;
use crate::core::Core;
use crate::data::{Load, fetch};
use crate::format::plural;
use crate::nav::PageEvent;
use crate::player_panels::switch;
use crate::settings_page::muted;
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, Accent};

/// Every site a plugin may talk to, each once: its mirrors first, then players and CDNs.
pub fn hosts_of(domains: &[String], allowed: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for h in domains
        .iter()
        .map(|d| {
            url::Url::parse(d)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
                .unwrap_or_else(|| d.clone())
        })
        .chain(allowed.iter().cloned())
    {
        if !h.is_empty() && !out.contains(&h) {
            out.push(h);
        }
    }
    out
}

/// "anilibria.top и ещё 3 сайта": the main site and how many more there are.
pub fn hosts_line(hosts: &[String]) -> String {
    match hosts {
        [] => String::new(),
        [one] => one.clone(),
        [first, rest @ ..] => {
            let more = rest.len() as i64;
            format!(
                "{first} и ещё {more} {}",
                plural(more, "сайт", "сайта", "сайтов")
            )
        }
    }
}

pub fn time_ago(ts: i64, now: i64) -> String {
    let s = (now - ts).max(0);
    match s {
        0..60 => "только что".into(),
        60..3600 => format!("{} мин назад", s / 60),
        3600..86400 => format!("{} ч назад", s / 3600),
        _ => format!("{} дн назад", s / 86400),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Ok,
    Warn,
    Bad,
    None,
}

impl Tone {
    fn color(self) -> Hsla {
        match self {
            Tone::Ok => rgb(theme::OK).into(),
            Tone::Warn => rgb(theme::WARN).into(),
            Tone::Bad => rgb(theme::DANGER).into(),
            Tone::None => Hsla::from(rgb(theme::TEXT_3)).opacity(0.6),
        }
    }
}

/// A source's week in a word and a line: its share of good answers and the latest failure.
pub fn health_of(h: &SourceHealth, now: i64) -> (Tone, &'static str, String) {
    let total = h.ok + h.failed;
    if total == 0 {
        return (
            Tone::None,
            "нет данных",
            "За неделю к источнику не обращались.".into(),
        );
    }
    let share = h.ok as f64 / total as f64;
    let (tone, label) = if share >= 0.8 {
        (Tone::Ok, "работает")
    } else if share >= 0.4 {
        (Tone::Warn, "с перебоями")
    } else {
        (Tone::Bad, "не работает")
    };
    let mut detail = format!("{} из {total} запросов за неделю", h.ok);
    if let (Some(kind), Some(at)) = (&h.last_error, h.last_error_at) {
        let what = match kind.as_str() {
            "Blocked" => "сайт недоступен",
            "GeoBlocked" => "закрыт в регионе",
            "Challenge" => "просит проверку на человека",
            "RateLimited" => "слишком много запросов",
            "Broken" => "ошибка источника",
            other => other,
        };
        detail += &format!(". Последний сбой: {what}, {}", time_ago(at, now));
    }
    (tone, label, detail)
}

pub struct Registry {
    url: String,
    pub entries: Vec<RegistryEntry>,
}

/// The registry the user chose, or the built-in one.
fn current_registry(core: &Core) -> String {
    let setting = db::get_setting(&core.db.lock().unwrap(), REGISTRY_SETTING)
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(str::to_string));
    registry_url(setting)
}

/// What the registry offers, each plugin marked with what is installed.
pub fn load_registry(core: &Arc<Core>) -> impl Future<Output = anyhow::Result<Registry>> + use<> {
    let url = current_registry(core);
    core.run(move |c| async move {
        let index = c.host.fetch_index(&url).await?;
        let entries = merge(&index, &c.host.installed()?);
        Ok(Registry { url, entries })
    })
}

/// What a third-party repository offers and which sites its plugins talk to, before installing.
pub fn preview_repo(
    core: &Arc<Core>,
    url: String,
) -> impl Future<Output = anyhow::Result<Registry>> + use<> {
    core.run(move |c| async move {
        let index = c.host.fetch_index(&url).await?;
        let entries = merge(&index, &c.host.installed()?);
        Ok(Registry { url, entries })
    })
}

/// Installs every plugin of a third-party repository.
pub fn install_repo(
    core: &Arc<Core>,
    url: String,
) -> impl Future<Output = anyhow::Result<()>> + use<> {
    core.run(move |c| async move {
        c.host.install_repo(&url, None).await?;
        c.resolver.reload_dubs(DubTable::load(&c.host.dubs_path()));
        Ok(())
    })
}

/// Installs or updates what the registry offers, or only `ids`.
pub fn install_from_registry(
    core: &Arc<Core>,
    ids: Option<Vec<String>>,
) -> impl Future<Output = anyhow::Result<()>> + use<> {
    let url = current_registry(core);
    core.run(move |c| async move {
        c.host.install_repo(&url, ids.as_deref()).await?;
        c.resolver.reload_dubs(DubTable::load(&c.host.dubs_path()));
        Ok(())
    })
}

pub struct PluginsPane {
    core: Arc<Core>,
    registry: Load<Registry>,
    health: Vec<SourceHealth>,
    /// Installed plugins not from the registry.
    others: Vec<Manifest>,
    /// What is being installed: one plugin's id, or "*" for all of them.
    busy: Option<String>,
    error: Option<String>,
    custom_open: bool,
    url: Entity<TextInput>,
    preview: Option<Load<Registry>>,
    installing_custom: bool,
    custom_error: Option<String>,
}

impl EventEmitter<PageEvent> for PluginsPane {}

impl PluginsPane {
    pub fn new(core: Arc<Core>, cx: &mut Context<Self>) -> Self {
        let url = cx.new(|cx| {
            let mut input = TextInput::new("https://.../index.json", cx);
            input.compact = true;
            input.search = false;
            input
        });
        cx.subscribe(&url, |pane, _, event: &InputEvent, cx| match event {
            InputEvent::Submit => pane.show_preview(cx),
            _ => {
                pane.preview = None;
                cx.notify();
            }
        })
        .detach();
        let mut pane = Self {
            core,
            registry: Load::Pending,
            health: Vec::new(),
            others: Vec::new(),
            busy: None,
            error: None,
            custom_open: false,
            url,
            preview: None,
            installing_custom: false,
            custom_error: None,
        };
        pane.reload(cx);
        pane
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        fetch(cx, load_registry(&self.core), |pane, registry| {
            pane.registry = registry
        });
        self.health = self.core.resolver.health_report().unwrap_or_default();
        self.others = self.core.host.list();
    }

    /// Runs a change to the plugins, then shows them as they are.
    fn change(
        &mut self,
        busy: Option<String>,
        job: impl Future<Output = anyhow::Result<()>> + 'static,
        cx: &mut Context<Self>,
    ) {
        self.busy = busy;
        self.error = None;
        cx.spawn(async move |pane, cx| {
            let got = job.await;
            pane.update(cx, |pane, cx| {
                pane.busy = None;
                if let Err(e) = got {
                    pane.error = Some(format!("{e:#}"));
                }
                pane.reload(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Installs or updates what the registry offers, or only `ids`.
    fn install(&mut self, ids: Option<Vec<String>>, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let busy = match ids.as_deref() {
            Some([one]) => one.clone(),
            _ => "*".into(),
        };
        let job = install_from_registry(&self.core, ids);
        self.change(Some(busy), job, cx);
    }

    fn show_preview(&mut self, cx: &mut Context<Self>) {
        let url = self.url.read(cx).text().trim().to_string();
        if url.is_empty() {
            return;
        }
        self.preview = Some(Load::Pending);
        self.custom_error = None;
        fetch(cx, preview_repo(&self.core, url), |pane, preview| {
            pane.preview = Some(preview)
        });
        cx.notify();
    }

    fn install_custom(&mut self, url: String, cx: &mut Context<Self>) {
        self.installing_custom = true;
        self.custom_error = None;
        let job = install_repo(&self.core, url);
        cx.spawn(async move |pane, cx| {
            let got = job.await;
            pane.update(cx, |pane, cx| {
                pane.installing_custom = false;
                match got {
                    Ok(()) => {
                        pane.preview = None;
                        pane.url.update(cx, |input, cx| input.set_text("", cx));
                    }
                    Err(e) => pane.custom_error = Some(format!("{e:#}")),
                }
                pane.reload(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn row(
        &self,
        e: &RegistryEntry,
        i: usize,
        last: bool,
        accent: Hsla,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let installed = e.installed_version.is_some();
        let hosts = hosts_of(&e.domains, &e.allowed_hosts);
        let health = self.health.iter().find(|h| h.extension_id == e.id);
        let state = health
            .filter(|_| installed && e.enabled)
            .map(|h| health_of(h, now()));
        let tone = state.as_ref().map_or(Tone::None, |s| s.0);
        let failing = state
            .as_ref()
            .filter(|s| matches!(s.0, Tone::Warn | Tone::Bad));
        let busy = self.busy.as_deref() == Some(e.id.as_str()) || self.busy.as_deref() == Some("*");
        let id = e.id.clone();
        let group: gpui::SharedString = format!("plugin-{i}").into();
        let actions: AnyElement = if installed {
            let enabled = e.enabled;
            let (update_id, remove_id, toggle_id) = (id.clone(), id.clone(), id.clone());
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .when(e.update_available, |d| {
                    d.child(
                        button(
                            ("update", i),
                            "Обновить",
                            None,
                            Variant::Glass,
                            Size::M,
                            accent,
                        )
                        .when(busy, |b| b.opacity(0.5))
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            pane.install(Some(vec![update_id.clone()]), cx)
                        })),
                    )
                })
                .child(
                    div()
                        .id(("remove", i))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(32.))
                        .rounded_full()
                        .invisible()
                        .group_hover(group.clone(), |s| s.visible())
                        .hover(|s| s.bg(rgba(0xff7a8a1f)))
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            let id = remove_id.clone();
                            let job = pane
                                .core
                                .run(move |c| async move { c.host.remove(&id).await });
                            pane.change(None, job, cx);
                        }))
                        .child(icon("close", 18.).text_color(rgb(theme::TEXT_3))),
                )
                .child(
                    switch(("plugin-on", i), enabled, accent, window, cx).on_click(cx.listener(
                        move |pane, _, _, cx| {
                            let id = toggle_id.clone();
                            let job = pane.core.run(move |c| async move {
                                c.host.set_enabled(&id, !enabled).await
                            });
                            pane.change(None, job, cx);
                        },
                    )),
                )
                .into_any_element()
        } else {
            button(
                ("connect", i),
                "Подключить",
                Some("plus"),
                Variant::Glass,
                Size::M,
                accent,
            )
            .when(busy, |b| b.opacity(0.5))
            .on_click(cx.listener(move |pane, _, _, cx| pane.install(Some(vec![id.clone()]), cx)))
            .into_any_element()
        };
        let version = e
            .installed_version
            .clone()
            .unwrap_or_else(|| e.version.clone());
        div()
            .group(group)
            .flex()
            .items_center()
            .gap(px(12.))
            .min_h(px(50.))
            .pl(px(16.))
            .pr(px(8.))
            .py(px(6.))
            .bg(rgba(0xffffff05))
            .hover(|s| s.bg(rgba(0xffffff0f)))
            .when(i > 0, |d| d.border_t_1().border_color(theme::line()))
            .when(i == 0, |d| d.rounded_t(theme::R_M))
            .when(last, |d| d.rounded_b(theme::R_M))
            .child(
                div()
                    .flex_none()
                    .size(px(8.))
                    .rounded_full()
                    .bg(tone.color()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(8.))
                            .min_w(px(0.))
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .whitespace_nowrap()
                                    .child(e.name.clone()),
                            )
                            .when(e.update_available, |d| {
                                d.child(
                                    div()
                                        .px(px(8.))
                                        .py(px(1.))
                                        .rounded_full()
                                        .bg(rgba(0xf0c86724))
                                        .text_color(rgb(theme::WARN))
                                        .text_size(px(11.))
                                        .font_weight(FontWeight::EXTRA_BOLD)
                                        .whitespace_nowrap()
                                        .child(format!("обновление {}", e.version)),
                                )
                            })
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(13.))
                                    .text_color(rgb(theme::TEXT_3))
                                    .child(format!("{}, v{version}", hosts_line(&hosts))),
                            ),
                    )
                    .children(failing.map(|(tone, label, detail)| {
                        div()
                            .mt(px(1.))
                            .text_size(px(12.))
                            .text_color(tone.color())
                            .child(format!("{label}: {detail}"))
                    })),
            )
            .child(actions)
            .into_any_element()
    }

    fn custom(&self, accent: Hsla, cx: &mut Context<Self>) -> Div {
        let open = self.custom_open;
        let head = div()
            .id("custom-repo")
            .cursor_pointer()
            .flex()
            .items_center()
            .gap(px(8.))
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(theme::TEXT_2))
            .hover(|s| s.text_color(rgb(theme::TEXT)))
            .on_click(cx.listener(|pane, _, _, cx| {
                pane.custom_open = !pane.custom_open;
                cx.notify();
            }))
            .child(
                icon("back", 16.)
                    .text_color(rgb(theme::TEXT_3))
                    .with_transformation(Transformation::rotate(radians(if open {
                        PI * 1.5
                    } else {
                        PI
                    }))),
            )
            .child("Свой репозиторий");
        let mut out = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .pt(px(12.))
            .border_t_1()
            .border_color(theme::line())
            .child(head);
        if !open {
            return out;
        }
        let error = |text: String| {
            div()
                .text_size(px(13.))
                .text_color(rgb(theme::DANGER))
                .child(text)
        };
        out = out
            .child(muted(
                "Ссылка на index.json стороннего репозитория плагинов или путь к нему на диске.",
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(div().flex_1().child(self.url.clone()))
                    .child(
                        button("preview", "Показать", None, Variant::Glass, Size::M, accent)
                            .when(matches!(self.preview, Some(Load::Pending)), |b| {
                                b.opacity(0.5)
                            })
                            .on_click(cx.listener(|pane, _, _, cx| pane.show_preview(cx))),
                    ),
            );
        match &self.preview {
            Some(Load::Failed(e)) => out = out.child(error(e.clone())),
            Some(Load::Ready(r)) if r.entries.is_empty() => {
                out = out.child(muted("В этом репозитории нет плагинов."))
            }
            Some(Load::Ready(r)) => {
                let n = r.entries.len() as i64;
                let url = r.url.clone();
                out = out
                    .child(
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
                                    .child(
                                        div().font_weight(FontWeight::BOLD).child(e.name.clone()),
                                    )
                                    .child(muted(format!("v{}", e.version)))
                                    .children(
                                        hosts_of(&e.domains, &e.allowed_hosts).into_iter().map(
                                            |h| {
                                                div()
                                                    .px(px(8.))
                                                    .py(px(1.))
                                                    .rounded_full()
                                                    .bg(theme::glass_strong())
                                                    .text_size(px(12.))
                                                    .child(h)
                                            },
                                        ),
                                    )
                            })),
                    )
                    .child(muted("Плагины будут ходить только на эти сайты."))
                    .child(
                        div().child(
                            button(
                                "install-custom",
                                format!(
                                    "Подключить {n} {}",
                                    plural(n, "плагин", "плагина", "плагинов")
                                ),
                                None,
                                Variant::Glass,
                                Size::M,
                                accent,
                            )
                            .when(self.installing_custom, |b| b.opacity(0.5))
                            .on_click(cx.listener(
                                move |pane, _, _, cx| {
                                    if !pane.installing_custom {
                                        pane.install_custom(url.clone(), cx)
                                    }
                                },
                            )),
                        ),
                    );
            }
            _ => {}
        }
        let registry_ids: Vec<&str> = match &self.registry {
            Load::Ready(r) => r.entries.iter().map(|e| e.id.as_str()).collect(),
            _ => Vec::new(),
        };
        out.children(self.custom_error.clone().map(error)).children(
            self.others
                .iter()
                .filter(|m| !registry_ids.contains(&m.id.as_str()))
                .map(|m| {
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(div().font_weight(FontWeight::BOLD).child(m.name.clone()))
                        .child(muted(format!("v{}", m.version)))
                }),
        )
    }
}

impl Render for PluginsPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        let entries: &[RegistryEntry] = match &self.registry {
            Load::Ready(r) => &r.entries,
            _ => &[],
        };
        let missing: Vec<String> = entries
            .iter()
            .filter(|e| e.installed_version.is_none())
            .map(|e| e.id.clone())
            .collect();
        let outdated: Vec<String> = entries
            .iter()
            .filter(|e| e.update_available)
            .map(|e| e.id.clone())
            .collect();
        let all_busy = self.busy.as_deref() == Some("*");
        let side: Option<AnyElement> = if !missing.is_empty() {
            Some(
                button(
                    "install-all",
                    "Подключить все",
                    Some("puzzle"),
                    Variant::Primary,
                    Size::M,
                    accent,
                )
                .when(all_busy, |b| b.opacity(0.6))
                .on_click(cx.listener(|pane, _, _, cx| pane.install(None, cx)))
                .into_any_element(),
            )
        } else if !outdated.is_empty() {
            Some(
                button(
                    "update-all",
                    "Обновить все",
                    Some("retry"),
                    Variant::Primary,
                    Size::M,
                    accent,
                )
                .when(all_busy, |b| b.opacity(0.6))
                .on_click(
                    cx.listener(move |pane, _, _, cx| pane.install(Some(outdated.clone()), cx)),
                )
                .into_any_element(),
            )
        } else if !entries.is_empty() {
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(theme::OK))
                    .whitespace_nowrap()
                    .child(icon("check", 18.).text_color(rgb(theme::OK)))
                    .child("Все подключены")
                    .into_any_element(),
            )
        } else {
            None
        };
        let head = div()
            .flex()
            .items_start()
            .justify_between()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child("Источники"),
                    )
                    .child(muted(
                        "Плагины, откуда берутся серии. Каждый в песочнице и ходит только на свои сайты.",
                    )),
            )
            .children(side);

        let list: AnyElement = match &self.registry {
            Load::Pending => div()
                .flex()
                .flex_col()
                .gap(px(1.))
                .children((0..2).map(|_| div().h(px(50.)).rounded(px(8.)).bg(theme::glass())))
                .into_any_element(),
            Load::Failed(e) => div()
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(16.))
                .py(px(12.))
                .rounded(theme::R_M)
                .bg(rgba(0xf0c86714))
                .child(icon("warn", 20.).text_color(rgb(theme::WARN)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.))
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .child("Реестр плагинов недоступен"),
                        )
                        .child(muted(e.clone())),
                )
                .child(
                    button(
                        "registry-retry",
                        "Повторить",
                        Some("retry"),
                        Variant::Glass,
                        Size::M,
                        accent,
                    )
                    .on_click(cx.listener(|pane, _, _, cx| {
                        pane.registry = Load::Pending;
                        pane.reload(cx);
                        cx.notify();
                    })),
                )
                .into_any_element(),
            Load::Ready(r) => {
                let n = r.entries.len();
                let rows: Vec<AnyElement> = r
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(i, e)| self.row(e, i, i + 1 == n, accent, window, cx))
                    .collect();
                div()
                    .flex()
                    .flex_col()
                    .rounded(theme::R_M)
                    .border_1()
                    .border_color(theme::line())
                    .children(rows)
                    .into_any_element()
            }
        };

        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .p(px(24.))
            .max_w(px(760.))
            .rounded(theme::R_L)
            .bg(theme::glass())
            .border_1()
            .border_color(theme::line())
            .child(head)
            .child(list)
            .children(self.error.clone().map(|e| {
                div()
                    .text_size(px(13.))
                    .text_color(rgb(theme::DANGER))
                    .child(e)
            }))
            .child(self.custom(accent, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugin_lists_every_site_once_and_names_the_main_one() {
        let domains = [
            "https://api.yani.tv".to_string(),
            "https://ru.yummyani.me/api".into(),
        ];
        let allowed = [
            "*.kodikplayer.com".to_string(),
            "video.sibnet.ru".into(),
            "api.yani.tv".into(),
        ];
        assert_eq!(
            hosts_of(&domains, &allowed),
            [
                "api.yani.tv",
                "ru.yummyani.me",
                "*.kodikplayer.com",
                "video.sibnet.ru"
            ]
        );
        let h = |n: usize| -> Vec<String> { (0..n).map(|i| format!("{i}.tv")).collect() };
        assert_eq!(hosts_line(&h(1)), "0.tv");
        assert_eq!(hosts_line(&h(2)), "0.tv и ещё 1 сайт");
        assert_eq!(hosts_line(&h(4)), "0.tv и ещё 3 сайта");
        assert_eq!(hosts_line(&h(6)), "0.tv и ещё 5 сайтов");
        assert_eq!(hosts_line(&[]), "");
    }

    #[test]
    fn a_source_is_working_flaky_or_down_by_its_week() {
        let now = 1_000_000;
        let h = |ok, failed, err: Option<&str>, at: Option<i64>| SourceHealth {
            extension_id: "x".into(),
            ok,
            failed,
            last_ok_at: None,
            last_error: err.map(str::to_string),
            last_error_at: at,
        };
        let (tone, label, detail) = health_of(&h(12, 1, None, None), now);
        assert_eq!(
            (tone, label, detail.as_str()),
            (Tone::Ok, "работает", "12 из 13 запросов за неделю")
        );
        assert_eq!(health_of(&h(3, 2, None, None), now).0, Tone::Warn);
        assert_eq!(health_of(&h(1, 9, None, None), now).1, "не работает");
        assert_eq!(health_of(&h(0, 0, None, None), now).0, Tone::None);
        assert_eq!(
            health_of(&h(3, 2, Some("GeoBlocked"), Some(now - 3 * 3600)), now).2,
            "3 из 5 запросов за неделю. Последний сбой: закрыт в регионе, 3 ч назад"
        );
        assert_eq!(time_ago(now - 30, now), "только что");
        assert_eq!(time_ago(now - 300, now), "5 мин назад");
        assert_eq!(time_ago(now - 2 * 86400, now), "2 дн назад");
    }
}
