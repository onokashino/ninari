//! The data section: how much the caches take and emptying them, and the library saved to a file
//! and added back from one.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    Context, EventEmitter, IntoElement, PathPromptOptions, Render, Window, div, prelude::*, px, rgb,
};
use ninari_core::catalog::calendar::civil_from_days;
use ninari_core::error::AppError;
use ninari_core::library::Backup;
use ninari_core::maintenance::{clear_dir, dir_size};

use crate::button::{Size, Variant, button};
use crate::clock::now;
use crate::core::{Core, cache_dir};
use crate::format::plural;
use crate::nav::PageEvent;
use crate::settings_page::{card, muted, setting_row, setting_text};
use crate::theme::{self, Accent};

const CACHES: [(&str, &str, &str); 2] = [
    (
        "posters",
        "Постеры и обложки",
        "Скачаются заново, когда снова понадобятся.",
    ),
    (
        "thumbs",
        "Превью перемотки",
        "Кадры над полосой перемотки, собираются во время просмотра.",
    ),
];

/// "17,4 МБ": kilobytes rounded up, larger units with one decimal below a hundred.
pub fn size_label(bytes: u64) -> String {
    const MB: f64 = (1u64 << 20) as f64;
    const GB: f64 = (1u64 << 30) as f64;
    if bytes < 1 << 20 {
        return format!("{} КБ", bytes.div_ceil(1024));
    }
    let (value, unit) = if bytes < 1 << 30 {
        (bytes as f64 / MB, "МБ")
    } else {
        (bytes as f64 / GB, "ГБ")
    };
    let shown = if value < 100. {
        format!("{}", (value * 10.).round() / 10.)
    } else {
        format!("{}", value.round())
    };
    format!("{} {unit}", shown.replace('.', ","))
}

fn restore_message(titles: usize, episodes: usize) -> String {
    if titles == 0 && episodes == 0 {
        return "Нечего добавлять: всё из копии уже есть в библиотеке.".into();
    }
    let (t, e) = (titles as i64, episodes as i64);
    format!(
        "Добавлено {t} {} и {e} {}.",
        plural(t, "тайтл", "тайтла", "тайтлов"),
        plural(e, "серия", "серии", "серий")
    )
}

pub struct DataPane {
    core: Arc<Core>,
    sizes: [u64; 2],
    busy: bool,
    note: Option<String>,
    error: Option<String>,
}

impl EventEmitter<PageEvent> for DataPane {}

impl DataPane {
    pub fn new(core: Arc<Core>, _cx: &mut Context<Self>) -> Self {
        let mut pane = Self {
            core,
            sizes: [0; 2],
            busy: false,
            note: None,
            error: None,
        };
        pane.measure();
        pane
    }

    fn measure(&mut self) {
        self.sizes = CACHES.map(|(kind, ..)| dir_size(&cache_dir().join(kind)));
    }

    /// Runs `job` on the core; what it says goes under the buttons, and the library may have
    /// changed.
    fn run(
        &mut self,
        job: impl Future<Output = anyhow::Result<Option<String>>> + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.note = None;
        self.error = None;
        cx.spawn(async move |pane, cx| {
            let got = job.await;
            pane.update(cx, |pane, cx| {
                pane.busy = false;
                match got {
                    Ok(Some(note)) => {
                        pane.note = Some(note);
                        cx.emit(PageEvent::Changed);
                    }
                    Ok(None) => {}
                    Err(e) => pane.error = Some(format!("{e:#}")),
                }
                pane.measure();
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn clear(&mut self, kind: &'static str, cx: &mut Context<Self>) {
        let job = self.core.run(move |_| async move {
            let freed = clear_dir(&cache_dir().join(kind))
                .map_err(|e| AppError::new("Io", e.to_string()))?;
            Ok(Some(format!("Освобождено {}.", size_label(freed))))
        });
        self.run(job, cx);
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let (y, m, d) = civil_from_days(now().div_euclid(86_400));
        let name = format!("ninari-backup-{y}-{m:02}-{d:02}.json");
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_default();
        let picked = cx.prompt_for_new_path(&home, Some(&name));
        let core = self.core.clone();
        cx.spawn(async move |pane, cx| {
            let Ok(Ok(Some(path))) = picked.await else {
                return;
            };
            pane.update(cx, |pane, cx| {
                let job = core.run(move |c| async move {
                    let backup = c.library.export()?;
                    std::fs::write(&path, serde_json::to_vec_pretty(&backup)?)
                        .map_err(|e| AppError::new("Io", e.to_string()))?;
                    Ok(Some(format!("Копия сохранена: {}", path.display())))
                });
                pane.run(job, cx);
            })
            .ok();
        })
        .detach();
    }

    fn restore(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Копия библиотеки".into()),
        });
        let core = self.core.clone();
        cx.spawn(async move |pane, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            pane.update(cx, |pane, cx| {
                let job = core.run(move |c| async move {
                    let text = std::fs::read_to_string(&path)
                        .map_err(|e| AppError::new("Io", e.to_string()))?;
                    let backup: Backup = serde_json::from_str(&text).map_err(|_| {
                        AppError::new("BadRequest", "в этом файле нет копии библиотеки ninari")
                    })?;
                    // Warming the catalog cache saves the library page hundreds of one-by-one
                    // requests; offline it fails, and the page then fetches the titles itself.
                    let ids: Vec<i64> = backup.library.iter().map(|e| e.shikimori_id).collect();
                    let _ = c.catalog.get_many(&ids).await;
                    let (titles, episodes) = c.library.restore(&backup)?;
                    Ok(Some(restore_message(titles, episodes)))
                });
                pane.run(job, cx);
            })
            .ok();
        })
        .detach();
    }
}

impl Render for DataPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        let mut out = card("Данные");
        for (i, (kind, title, hint)) in CACHES.into_iter().enumerate() {
            let size = self.sizes[i];
            out = out.child(setting_row(
                setting_text(title, hint),
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(muted(size_label(size)))
                    .child(
                        button(
                            ("clear", i),
                            "Очистить",
                            None,
                            Variant::Glass,
                            Size::M,
                            accent,
                        )
                        .when(size == 0 || self.busy, |b| b.opacity(0.5))
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            if size > 0 {
                                pane.clear(kind, cx)
                            }
                        })),
                    ),
            ));
        }
        out.child(setting_text(
            "Копия библиотеки",
            "Список, статусы, озвучки и серии в одном файле. Восстановление добавляет только то, чего ещё нет.",
        ))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(12.))
                .child(
                    button("backup-save", "Сохранить копию", None, Variant::Glass, Size::M, accent)
                        .when(self.busy, |b| b.opacity(0.5))
                        .on_click(cx.listener(|pane, _, _, cx| pane.save(cx))),
                )
                .child(
                    button("backup-restore", "Восстановить из копии", None, Variant::Glass, Size::M, accent)
                        .when(self.busy, |b| b.opacity(0.5))
                        .on_click(cx.listener(|pane, _, _, cx| pane.restore(cx))),
                ),
        )
        .children(self.note.clone().map(muted))
        .children(self.error.clone().map(|e| {
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
    fn sizes_read_like_a_file_manager() {
        assert_eq!(size_label(0), "0 КБ");
        assert_eq!(size_label(1500), "2 КБ");
        assert_eq!(size_label(17_400_000), "16,6 МБ");
        assert_eq!(size_label(150 << 20), "150 МБ");
        assert_eq!(size_label(3 << 30), "3 ГБ");
    }

    #[test]
    fn a_restore_says_what_it_added() {
        assert_eq!(
            restore_message(0, 0),
            "Нечего добавлять: всё из копии уже есть в библиотеке."
        );
        assert_eq!(restore_message(2, 5), "Добавлено 2 тайтла и 5 серий.");
    }
}
