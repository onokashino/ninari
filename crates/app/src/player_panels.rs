//! The player's side sheets: settings, dubs, and episodes with the franchise's seasons; and the
//! switch they use.

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Div, ElementId, FontWeight, Hsla, SharedString, Window, div, prelude::*, px,
    rgb, rgba, white,
};
use ninari_core::catalog::details::Part;
use ninari_core::catalog::jikan::EpisodeMark;
use ninari_core::feeds::{AnimeCard, ContinueAt};
use ninari_core::library::EpisodeProgress;
use ninari_core::player::PlayerState;
use ninari_core::player::audio::AudioFx;
use ninari_core::player::upscale::{Flavor, Method, Mode, Tier};
use ninari_core::sources::TitleSources;

use crate::assets::icon;
use crate::episode_grid::chip;
use crate::episodes::{TileState, episode_tiles};
use crate::format::plural;
use crate::franchise::part_chips;
use crate::motion;
use crate::popover::Menu;
use crate::select::{SelectOption, select};
use crate::theme::{self, SPRING};

/// Speeds on offer in the sheet.
const SPEEDS: [f64; 5] = [0.75, 1., 1.25, 1.5, 2.];

/// Sound effects: (name, hint, is on, turn it over).
type Effect = (
    &'static str,
    &'static str,
    fn(&AudioFx) -> bool,
    fn(&mut AudioFx),
);

const EFFECTS: [Effect; 5] = [
    (
        "Ровная громкость",
        "Тихие реплики громче, взрывы тише. Серии и озвучки звучат на одном уровне.",
        |f| f.even,
        |f| f.even = !f.even,
    ),
    (
        "Чёткие голоса",
        "Голос выходит вперёд, музыка и шум остаются фоном.",
        |f| f.voice,
        |f| f.voice = !f.voice,
    ),
    (
        "Ночной режим",
        "Громкие моменты не бьют по ушам, можно слушать тихо.",
        |f| f.night,
        |f| f.night = !f.night,
    ),
    (
        "Шумодав",
        "Убирает шипение и гул домашней записи озвучки.",
        |f| f.denoise,
        |f| f.denoise = !f.denoise,
    ),
    (
        "Наушники",
        "Мягче разводит каналы, уши меньше устают.",
        |f| f.headphones,
        |f| f.headphones = !f.headphones,
    ),
];

/// The delay one step (50 ms) earlier (-1) or later (+1), within five seconds.
pub fn nudge_delay(delay: f64, direction: f64) -> f64 {
    ((delay + direction * 0.05) * 1000.).round() / 1000.
}

/// "+150 мс", "0 мс".
pub fn delay_label(delay: f64) -> String {
    let ms = (delay * 1000.).round() as i64;
    match ms {
        0 => "0 мс".into(),
        ms if ms > 0 => format!("+{ms} мс"),
        ms => format!("-{} мс", -ms),
    }
}

/// What the settings sheet can change.
#[derive(Clone, Copy)]
pub enum Change {
    Speed(f64),
    /// "opening", "ending" or "fillers".
    Skip(&'static str),
    Effects(AudioFx),
    Delay(f64),
    Track(i64),
}

pub type OnChange = Rc<dyn Fn(Change, &mut Window, &mut App)>;

/// An on/off switch whose knob springs across.
pub fn switch(
    id: impl Into<ElementId>,
    on: bool,
    accent: Hsla,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Stateful<Div> {
    let id = id.into();
    motion::set(id.clone(), on, Duration::from_millis(320), SPRING, cx);
    let t = motion::get(id.clone(), window, cx);
    div()
        .id(id)
        .cursor_pointer()
        .relative()
        .flex_none()
        .w(px(44.))
        .h(px(26.))
        .rounded_full()
        .border_1()
        .border_color(theme::line())
        .bg(theme::mix(rgba(0xffffff1f), accent, t.clamp(0., 1.)))
        .child(
            div()
                .absolute()
                .top(px(2.))
                .left(px(2. + 18. * t))
                .size(px(20.))
                .rounded_full()
                .bg(theme::mix(rgb(theme::TEXT_2), white(), t.clamp(0., 1.)))
                .shadow(vec![gpui::BoxShadow {
                    color: rgba(0x00000059).into(),
                    offset: gpui::point(px(0.), px(2.)),
                    blur_radius: px(6.),
                    spread_radius: px(0.),
                    inset: false,
                }]),
        )
}

fn head(text: &'static str) -> Div {
    div()
        .text_size(px(13.))
        .font_weight(FontWeight::BOLD)
        .text_color(rgba(0xffffff8c))
        .child(text)
}

fn hint(text: impl Into<SharedString>) -> Div {
    div()
        .text_color(rgba(0xffffff8c))
        .text_size(px(12.))
        .line_height(px(17.))
        .child(text.into())
}

/// A row with a name, a hint under it, and a switch.
fn toggle_row(
    id: impl Into<ElementId>,
    name: &'static str,
    note: &'static str,
    on: bool,
    accent: Hsla,
    flip: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let id = id.into();
    div()
        .flex()
        .justify_between()
        .items_center()
        .gap(px(12.))
        .px(px(12.))
        .py(px(10.))
        .rounded(theme::R_M)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .min_w(px(0.))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
                .child(hint(note)),
        )
        .child(switch(id, on, accent, window, cx).on_click(move |_, window, cx| flip(window, cx)))
}

pub fn settings(
    s: &PlayerState,
    accent: Hsla,
    on_change: OnChange,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let pick = |c: Change| {
        let on_change = on_change.clone();
        move |window: &mut Window, cx: &mut App| on_change(c, window, cx)
    };

    let speeds = div()
        .flex()
        .flex_wrap()
        .gap(px(6.))
        .children(SPEEDS.map(|v| {
            let go = pick(Change::Speed(v));
            chip(
                ("speed", (v * 100.) as u64),
                (s.speed - v).abs() < 0.01,
                accent,
            )
            .on_click(move |_, window, cx| go(window, cx))
            .child(format!("{}×", v.to_string().replace('.', ",")))
        }));
    let skips = [
        (
            "opening",
            "Пропускать опенинги",
            "Там, где источник знает, где опенинг.",
            s.skip_opening,
        ),
        (
            "ending",
            "Пропускать эндинги",
            "Если после титров есть сцена, она останется.",
            s.skip_ending,
        ),
        (
            "fillers",
            "Пропускать филлеры",
            "Следующей включится серия по сюжету. Филлеры и пересказы по данным MyAnimeList.",
            s.skip_fillers,
        ),
    ];
    let mut play_rows: Vec<AnyElement> = Vec::new();
    for (part, name, note, on) in skips {
        let go = pick(Change::Skip(part));
        play_rows.push(
            toggle_row(
                SharedString::from(format!("skip-{part}")),
                name,
                note,
                on,
                accent,
                go,
                window,
                cx,
            )
            .into_any_element(),
        );
    }
    let mut fx_rows: Vec<AnyElement> = Vec::new();
    for (i, (name, note, is_on, flip)) in EFFECTS.into_iter().enumerate() {
        let mut next = s.audio_fx;
        flip(&mut next);
        let go = pick(Change::Effects(next));
        fx_rows.push(
            toggle_row(
                ("fx", i),
                name,
                note,
                is_on(&s.audio_fx),
                accent,
                go,
                window,
                cx,
            )
            .into_any_element(),
        );
    }

    let step = |id: &'static str, label: &'static str, to: f64| {
        let go = pick(Change::Delay(to));
        div()
            .id(id)
            .cursor_pointer()
            .flex_1()
            .flex()
            .justify_center()
            .px(px(12.))
            .py(px(9.))
            .rounded(theme::R_M)
            .bg(rgba(0xffffff14))
            .hover(|st| st.bg(rgba(0xffffff24)))
            .font_weight(FontWeight::SEMIBOLD)
            .on_click(move |_, window, cx| go(window, cx))
            .child(label)
    };
    let sync = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(head("Синхронизация"))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(step("earlier", "Раньше", nudge_delay(s.audio_delay, -1.)))
                .child(
                    div()
                        .min_w(px(84.))
                        .flex()
                        .justify_center()
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .child(delay_label(s.audio_delay)),
                )
                .child(step("later", "Позже", nudge_delay(s.audio_delay, 1.))),
        )
        .child(hint(format!(
            "Голос опережает губы: «Позже», отстаёт: «Раньше». Клавиши [ и ]. Сдвиг запоминается для озвучки {}.",
            s.dub_name
        )))
        .when(s.audio_delay != 0., |d| {
            let go = pick(Change::Delay(0.));
            d.child(
                div().flex().child(
                    div()
                        .id("reset")
                        .cursor_pointer()
                        .px(px(12.))
                        .py(px(9.))
                        .rounded(theme::R_M)
                        .bg(rgba(0xffffff14))
                        .hover(|st| st.bg(rgba(0xffffff24)))
                        .font_weight(FontWeight::SEMIBOLD)
                        .on_click(move |_, window, cx| go(window, cx))
                        .child("Сбросить"),
                ),
            )
        });

    let tracks = (s.audio_tracks.len() > 1).then(|| {
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(head("Дорожка"))
            .children(s.audio_tracks.iter().map(|t| {
                let picked = s.audio_track == Some(t.id);
                let go = pick(Change::Track(t.id));
                div()
                    .id(("track", t.id as u64))
                    .cursor_pointer()
                    .flex()
                    .justify_between()
                    .items_center()
                    .px(px(12.))
                    .py(px(10.))
                    .rounded(theme::R_M)
                    .font_weight(FontWeight::SEMIBOLD)
                    .map(|d| {
                        if picked {
                            d.bg(accent.opacity(0.16)).text_color(accent)
                        } else {
                            d.hover(|st| st.bg(rgba(0xffffff14)))
                        }
                    })
                    .on_click(move |_, window, cx| go(window, cx))
                    .child(t.label.clone())
                    .when(picked, |d| d.child(icon("check", 18.).text_color(accent)))
            }))
    });

    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(head("Воспроизведение"))
                .child(speeds)
                .child(div().flex().flex_col().gap(px(4.)).children(play_rows)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(head("Звук"))
                .child(div().flex().flex_col().gap(px(4.)).children(fx_rows)),
        )
        .child(sync)
        .children(tracks)
}

/// A small quality mark: green from 1080p, yellow below.
pub type OnControl = Rc<dyn Fn(&'static str, f64, &mut Window, &mut App)>;

thread_local! {
    /// The compare button is held: only then does a release anywhere bring the upscale back.
    static HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

const MODE_HINTS: [&str; 4] = [
    "По качеству серии: 1080p → C, 720p → A, ниже → A+A.",
    "Восстанавливает размытые и пережатые серии. Подходит большинству.",
    "Мягче A: меньше ореолов вокруг линий.",
    "Для чистых исходников: только увеличивает, ничего не восстанавливает.",
];
const FLAVOR_HINTS: [&str; 3] = [
    "Аккуратно удваивает картинку как есть.",
    "Убирает шум и добавляет резкости.",
    "Убирает шум и сглаживает.",
];
/// ArtCNN's shaders run only on a picture enlarged this much or more.
const ARTCNN_NOTE: &str = "Работает, когда картинка увеличена хотя бы в 1,3 раза: на весь экран почти всегда, в маленьком окне может не включиться.";

/// The upscaler, tuned while watching (opened by the sparkle): what runs now, the method,
/// Anime4K's mode or ArtCNN's flavor, the tier, and the original held up for comparison.
pub fn upscale_panel(
    s: &PlayerState,
    accent: Hsla,
    on_control: OnControl,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let u = s.upscale_cfg;
    let send = |action: &'static str, value: f64| {
        let on_control = on_control.clone();
        move |window: &mut Window, cx: &mut App| on_control(action, value, window, cx)
    };
    let chips = |key: &'static str,
                 action: &'static str,
                 options: &[(&'static str, bool)],
                 offset: usize| {
        div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .children(options.iter().enumerate().map(|(i, (label, on))| {
                let go = send(action, (i + offset) as f64);
                chip((key, i), *on, accent)
                    .on_click(move |_, window, cx| go(window, cx))
                    .child(*label)
            }))
    };
    let artcnn = u.method == Method::Artcnn;
    let methods = chips(
        "upscale-method",
        "upscale_method",
        &[("Anime4K", !artcnn), ("ArtCNN", artcnn)],
        0,
    );
    let (variant, variant_hint) = if artcnn {
        let flavors = [Flavor::Neutral, Flavor::Sharp, Flavor::Soft];
        let i = flavors.iter().position(|f| *f == u.flavor).unwrap_or(0);
        (
            chips(
                "upscale-flavor",
                "upscale_flavor",
                &[("Обычный", i == 0), ("Чётче", i == 1), ("Мягче", i == 2)],
                0,
            ),
            FLAVOR_HINTS[i],
        )
    } else {
        let modes = [Mode::Auto, Mode::A, Mode::B, Mode::C];
        let i = modes.iter().position(|m| *m == u.mode).unwrap_or(0);
        (
            chips(
                "upscale-mode",
                "upscale_mode",
                &[
                    ("Авто", i == 0),
                    ("A", i == 1),
                    ("B", i == 2),
                    ("C", i == 3),
                ],
                0,
            ),
            MODE_HINTS[i],
        )
    };
    let tier = |t: Tier| u.tier == t;
    let tiers = if artcnn {
        chips(
            "upscale-tier",
            "upscale_tier",
            &[
                ("Лёгкий", tier(Tier::Fast)),
                ("Полный", tier(Tier::Hq) || tier(Tier::Ultra)),
            ],
            1,
        )
    } else {
        chips(
            "upscale-tier",
            "upscale_tier",
            &[
                ("Fast", tier(Tier::Fast)),
                ("HQ", tier(Tier::Hq)),
                ("Ultra", tier(Tier::Ultra)),
            ],
            1,
        )
    };
    let flip = send("toggle_upscale", 0.);
    let (press, release, release_out) = (
        send("upscale_compare", 1.),
        send("upscale_compare", 0.),
        send("upscale_compare", 0.),
    );
    let section = |name: &'static str, body: Div| {
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(12.))
            .child(head(name))
            .child(body)
    };
    div()
        .w(px(360.))
        .flex()
        .flex_col()
        .gap(px(14.))
        .pb(px(12.))
        .child(
            div()
                .flex()
                .justify_between()
                .items_center()
                .gap(px(12.))
                .px(px(12.))
                .pt(px(10.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .child("Улучшение картинки"),
                        )
                        .child(hint(match &s.upscale_label {
                            Some(l) => format!("Сейчас: {l}"),
                            None => "Выключено".into(),
                        })),
                )
                .child(
                    switch("upscale-on", s.upscale, accent, window, cx)
                        .on_click(move |_, window, cx| flip(window, cx)),
                ),
        )
        .child(section("Метод", methods))
        .child(section(
            if artcnn {
                "Характер"
            } else {
                "Режим"
            },
            variant
                .child(div().w_full().child(hint(variant_hint)))
                .when(artcnn, |d| d.child(div().w_full().child(hint(ARTCNN_NOTE)))),
        ))
        .child(section(
            if artcnn {
                "Размер сети"
            } else {
                "Сила"
            },
            tiers,
        ))
        .child(
            div().px(px(12.)).child(
                div()
                    .id("upscale-compare")
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(14.))
                    .py(px(10.))
                    .rounded(theme::R_M)
                    .bg(theme::glass())
                    .border_1()
                    .border_color(theme::line())
                    .text_size(px(13.))
                    .font_weight(FontWeight::BOLD)
                    .hover(|st| st.bg(theme::glass_strong()))
                    .active(|st| st.bg(accent.opacity(0.25)))
                    .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                        HELD.set(true);
                        press(window, cx)
                    })
                    .on_mouse_up(gpui::MouseButton::Left, move |_, window, cx| {
                        if HELD.replace(false) {
                            release(window, cx)
                        }
                    })
                    .on_mouse_up_out(gpui::MouseButton::Left, move |_, window, cx| {
                        if HELD.replace(false) {
                            release_out(window, cx)
                        }
                    })
                    .child("Удерживай, чтобы сравнить с оригиналом"),
            ),
        )
        .child(
            div()
                .px(px(12.))
                .mt(px(-8.))
                .child(hint("Или держи клавишу C.")),
        )
}

pub fn quality_badge(quality: Option<u32>) -> Option<Div> {
    let q = quality?;
    let (bg, fg) = if q >= 1080 {
        (rgba(0x7fe39a24), rgb(0x7fe39a))
    } else {
        (rgba(0xf0c86724), rgb(0xf0c867))
    };
    Some(
        div()
            .px(px(6.))
            .py(px(1.))
            .rounded_full()
            .bg(bg)
            .text_color(fg)
            .text_size(px(11.))
            .font_weight(FontWeight::EXTRA_BOLD)
            .child(format!("{q}p")),
    )
}

/// The dubs of the title; one without this episode cannot be picked.
pub fn dubs(
    sources: Option<&TitleSources>,
    s: &PlayerState,
    accent: Hsla,
    pick: Rc<dyn Fn(String, &mut Window, &mut App)>,
) -> Div {
    let Some(sources) = sources else {
        return hint("Загружаем озвучки");
    };
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .children(sources.dubs.iter().map(|d| {
            let current = d.key == s.dub_key;
            let has = d.episodes.contains(&s.episode);
            let (pick, key) = (pick.clone(), d.key.clone());
            div()
                .id(SharedString::from(format!("dub-{}", d.key)))
                .flex()
                .justify_between()
                .items_center()
                .gap(px(12.))
                .px(px(14.))
                .py(px(12.))
                .rounded(theme::R_M)
                .font_weight(FontWeight::SEMIBOLD)
                .map(|row| {
                    if current {
                        row.bg(accent.opacity(0.16)).text_color(accent)
                    } else if has {
                        row.cursor_pointer()
                            .hover(|st| st.bg(rgba(0xffffff14)))
                            .on_click(move |_, window, cx| pick(key.clone(), window, cx))
                    } else {
                        row.opacity(0.4)
                    }
                })
                .child(d.name.clone())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .children(quality_badge(d.max_quality))
                        .when(current, |r| r.child(icon("check", 18.).text_color(accent))),
                )
        }))
}

/// What the episodes sheet knows of one part of the franchise.
#[derive(Default)]
pub struct PartData {
    pub card: Option<AnimeCard>,
    pub sources: Option<TitleSources>,
    pub progress: Vec<EpisodeProgress>,
    pub cont: Option<ContinueAt>,
    pub chosen_dub: Option<String>,
    pub marks: Vec<EpisodeMark>,
}

/// The seasons of the franchise, the dub of the season shown, and its episodes in five columns.
pub fn episodes(
    parts: &[Part],
    shown: i64,
    data: Option<&PartData>,
    picked_dub: Option<&str>,
    s: &PlayerState,
    accent: Hsla,
    width: f32,
    on_part: Rc<dyn Fn(i64, &mut Window, &mut App)>,
    on_dub: Rc<dyn Fn(String, &mut Window, &mut App)>,
    dub_menu: &Menu,
    on_play: Rc<dyn Fn(i64, i64, Option<String>, &mut Window, &mut App)>,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let here = shown == s.shikimori_id;
    let chips = part_chips(parts, s.shikimori_id);
    let seasons = (!chips.is_empty()).then(|| {
        div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .children(chips.into_iter().map(|c| {
                let go = on_part.clone();
                let id = c.id;
                chip(("part", c.id as u64), c.id == shown, accent)
                    .on_click(move |_, window, cx| go(id, window, cx))
                    .child(c.label)
            }))
    });
    let Some(data) = data.filter(|d| d.card.is_some() || d.sources.is_some()) else {
        return div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .children(seasons)
            .child(hint("Загружаем серии"));
    };
    let dubs = data.sources.as_ref().map_or(&[][..], |s| &s.dubs[..]);
    let playing = dubs.iter().find(|d| d.key == s.dub_key);
    let auto = playing
        .or_else(|| {
            dubs.iter()
                .find(|d| Some(&d.key) == data.chosen_dub.as_ref())
        })
        .or(dubs.first());
    let dub = picked_dub
        .and_then(|k| dubs.iter().find(|d| d.key == k))
        .or(auto);

    let mut notes: Vec<Div> = Vec::new();
    if !here
        && data.sources.is_some()
        && playing.is_none()
        && picked_dub.is_none()
        && let Some(d) = dub
    {
        notes.push(hint(format!(
            "В этом сезоне нет озвучки «{}», серии в «{}».",
            s.dub_name, d.name
        )));
    }
    let status = data.card.as_ref().and_then(|c| c.anime.status.as_deref());
    if status == Some("anons") {
        notes.push(hint("Этот сезон ещё не вышел."));
    } else if data.sources.is_some() && dubs.is_empty() {
        notes.push(hint("Этот сезон не нашёлся ни в одном источнике."));
    }

    let dub_choice = (dubs.len() > 1).then(|| {
        let options: Vec<SelectOption> = dubs
            .iter()
            .map(|d| {
                let count = d.episodes.len() as i64;
                SelectOption {
                    value: d.key.clone(),
                    label: d.name.clone(),
                    hint: Some(format!(
                        "{count} {}",
                        plural(count, "серия", "серии", "серий")
                    )),
                }
            })
            .collect();
        let value = dub.map_or_else(String::new, |d| d.key.clone());
        let pick = on_dub.clone();
        div().flex().child(select(
            "sheet-dub",
            dub_menu,
            &value,
            &options,
            accent,
            Rc::new(move |key, window, cx| pick(key, window, cx)),
            window,
            cx,
        ))
    });

    let tiles = data.card.as_ref().map_or(Vec::new(), |c| {
        let aired = if c.anime.status.as_deref() == Some("released") {
            c.anime.episodes
        } else {
            c.anime.episodes_aired
        };
        episode_tiles(
            c.anime.episodes,
            aired,
            dub.map_or(&[][..], |d| &d.episodes[..]),
            &data.progress,
            if here {
                Some(s.episode)
            } else {
                data.cont.as_ref().map(|c| c.episode)
            },
        )
    });
    let tile_w = ((width - 6. * 4.) / 5.).floor();
    let grid = div()
        .flex()
        .flex_wrap()
        .gap(px(6.))
        .children(tiles.iter().map(|t| {
            let open = !matches!(t.state, TileState::Missing | TileState::Upcoming);
            let key = ("sheet-tile", t.n as u64);
            let lift = if open {
                motion::get(key, window, cx)
            } else {
                0.
            };
            let mark = data.marks.iter().find(|m| m.number == t.n).and_then(|m| {
                if m.filler {
                    Some(0xf0c867)
                } else if m.recap {
                    Some(theme::TEXT_3)
                } else {
                    None
                }
            });
            let (go, n, part, dub_key) = (on_play.clone(), t.n, shown, dub.map(|d| d.key.clone()));
            div()
                .id(key)
                .relative()
                .top(px(-2. * lift))
                .w(px(tile_w))
                .h(px(44.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(12.))
                .font_weight(FontWeight::BOLD)
                .map(|d| match t.state {
                    TileState::Current => {
                        d.bg(accent.opacity(0.24)).border_2().border_color(accent)
                    }
                    TileState::Watched => d.bg(rgba(0xffffff0f)).text_color(theme::mix(
                        rgb(theme::TEXT_3),
                        accent,
                        0.65,
                    )),
                    TileState::Missing | TileState::Upcoming => d.bg(rgba(0xffffff0f)).opacity(0.3),
                    TileState::Available => d.bg(rgba(0xffffff0f)),
                })
                .when(open, |d| {
                    d.cursor_pointer()
                        .hover(|st| st.bg(rgba(0xffffff24)))
                        .on_hover(move |hovered, window, cx| {
                            motion::set(key, *hovered, theme::T_FAST, theme::EASE, cx);
                            window.refresh();
                        })
                        .on_click(move |_, window, cx| go(part, n, dub_key.clone(), window, cx))
                })
                .child(t.n.to_string())
                .children(mark.map(|c| {
                    div()
                        .absolute()
                        .top(px(5.))
                        .right(px(5.))
                        .size(px(6.))
                        .rounded_full()
                        .bg(rgb(c))
                }))
        }));

    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .children(seasons)
        .children(dub_choice)
        .children(notes)
        .child(grid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_delay_moves_in_fifty_milliseconds_and_reads_signed() {
        assert_eq!(nudge_delay(0., 1.), 0.05);
        assert_eq!(nudge_delay(0.05, -1.), 0.);
        assert_eq!(delay_label(0.15), "+150 мс");
        assert_eq!(delay_label(-0.05), "-50 мс");
        assert_eq!(delay_label(0.), "0 мс");
    }
}
