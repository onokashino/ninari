//! The player screen. mpv draws the video under the transparent window; this screen draws the
//! controls over it and drives ninari_core's player.

use std::cell::Cell;
use std::collections::HashMap;
use std::f32::consts::PI;
use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    Animation, AnimationExt, AnyElement, Bounds, BoxShadow, ClickEvent, Context, Div, EventEmitter,
    FocusHandle, FontWeight, Hsla, IntoElement, KeyDownEvent, KeyUpEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, SharedString, Stateful, Task,
    Transformation, Window, canvas, div, img, linear_color_stop, linear_gradient, point,
    prelude::*, px, radians, relative, rgb, rgba, svg, white,
};
use ninari_core::catalog::Anime;
use ninari_core::catalog::details::TitleDetails;
use ninari_core::catalog::posters::Palette;
use ninari_core::error::AppError;
use ninari_core::feeds;
use ninari_core::player::thumbs::Thumb;
use ninari_core::player::{PlayerEvent, PlayerState};
use ninari_core::sources::TitleSources;

use crate::assets::icon;
use crate::clock::next_episode_label;
use crate::core::Core;
use crate::franchise::next_season;
use crate::motion;
use crate::player_panels::{self, Change, PartData, quality_badge};
use crate::popover::{Menu, popover_up};
use crate::theme::{self, Accent, Curve, EASE};

/// What to play.
#[derive(Clone)]
pub struct Request {
    pub id: i64,
    pub episode: i64,
    /// The dub to play in; the one the title remembers, or the best, when `None`.
    pub dub_key: Option<String>,
    /// Where to start; the place the episode was left at when `None`.
    pub start: Option<f64>,
    pub title: SharedString,
    pub palette: Palette,
}

/// The player was left; its owner stops playback and drops it.
pub struct Closed;

#[derive(Clone, Copy, PartialEq)]
enum Panel {
    Episodes,
    Dubs,
    Settings,
}

impl Panel {
    fn title(self) -> &'static str {
        match self {
            Panel::Episodes => "Серии",
            Panel::Dubs => "Озвучка",
            Panel::Settings => "Настройки",
        }
    }
}

/// The controls go away after this long without the mouse moving.
const HIDE_AFTER: Duration = Duration::from_millis(2600);
/// A skipped opening can be brought back for this long.
const SKIPPED_SHOWN: Duration = Duration::from_secs(6);
const TOAST_SHOWN: Duration = Duration::from_secs(6);
/// The sheet and the pills come in with a little overshoot.
const SLIDE: Curve = Curve(0.22, 1.2, 0.36, 1.);
const SHEET_W: f32 = 360.;
/// mpv's `volume-max`: the slider's right end.
const MAX_VOLUME: f64 = 130.;

/// "1:05", "1:02:03".
pub fn clock(secs: f64) -> String {
    let s = secs.max(0.) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Which helper buttons show over the video: skip the opening while in it, the next episode in
/// the credits.
fn overlays(s: &PlayerState) -> (bool, bool) {
    let skip_opening = s
        .opening
        .is_some_and(|(from, to)| s.position >= from && s.position < to - 3.);
    let in_credits = match s.ending {
        Some((from, _)) => s.position >= from,
        None => s.duration > 0. && s.duration - s.position <= 90.,
    };
    (skip_opening, s.next.is_some() && in_credits)
}

/// The preview frame for a time on the seek bar: the last one at or before it.
fn thumb_at(thumbs: &[Thumb], t: f64) -> Option<&Thumb> {
    thumbs.iter().take_while(|th| th.t <= t).last()
}

/// A key's own meaning in the Russian layout too: "а" is F there, "х" is [.
fn latin(key: &str) -> &str {
    match key {
        "л" => "k",
        "а" => "f",
        "ь" => "m",
        "т" => "n",
        "з" => "p",
        "г" => "u",
        "я" => "z",
        "х" => "[",
        "ъ" => "]",
        other => other,
    }
}

pub struct Player {
    core: Arc<Core>,
    request: Request,
    state: Option<PlayerState>,
    error: Option<String>,
    /// The last available episode that played to its end.
    ended: Option<(i64, i64)>,
    skipped: Option<(&'static str, f64, Instant)>,
    thumbs: Vec<Thumb>,
    toast: Option<(String, Instant)>,
    /// A frame has been drawn: until then black covers the transparent window.
    frame_shown: bool,
    panel: Option<Panel>,
    quality_menu: Menu,
    upscale_menu: Menu,
    sheet_dub_menu: Menu,
    moved_at: Instant,
    /// A single click waits a moment in case it is a double click.
    pending_click: Option<Task<()>>,
    timers: Vec<Task<()>>,
    focus: FocusHandle,
    seek_bounds: Rc<Cell<Bounds<Pixels>>>,
    seek_hover: Option<f64>,
    seek_drag: Rc<Cell<Option<f64>>>,
    /// Where a seek was let go and when: shown until mpv gets there, so the knob does not
    /// jump back to where it was meanwhile.
    seek_to: Rc<Cell<Option<(f64, Instant)>>>,
    volume_bounds: Rc<Cell<Bounds<Pixels>>>,
    volume_drag: Rc<Cell<bool>>,
    /// The title playing: its card, franchise and dubs.
    title_id: i64,
    anime: Option<Anime>,
    details: Option<TitleDetails>,
    sources: Option<TitleSources>,
    /// The season the episodes sheet shows, the dub picked there per season, what is known.
    sheet_part: i64,
    sheet_dub: HashMap<i64, String>,
    parts: HashMap<i64, PartData>,
}

impl EventEmitter<Closed> for Player {}

impl Player {
    pub fn new(
        core: Arc<Core>,
        request: Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut player = Self::blank(core, request, window, cx);
        let r = player.request.clone();
        player.play_at(r.id, r.episode, r.dub_key, r.start, cx);
        player
    }

    fn blank(
        core: Arc<Core>,
        request: Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut player = Player {
            core,
            title_id: request.id,
            sheet_part: request.id,
            request,
            state: None,
            error: None,
            ended: None,
            skipped: None,
            thumbs: Vec::new(),
            toast: None,
            frame_shown: false,
            panel: None,
            quality_menu: Menu::default(),
            upscale_menu: Menu::default(),
            sheet_dub_menu: Menu::default(),
            moved_at: Instant::now(),
            pending_click: None,
            timers: Vec::new(),
            focus,
            seek_bounds: Rc::default(),
            seek_hover: None,
            seek_drag: Rc::default(),
            seek_to: Rc::default(),
            volume_bounds: Rc::default(),
            volume_drag: Rc::default(),
            anime: None,
            details: None,
            sources: None,
            sheet_dub: HashMap::new(),
            parts: HashMap::new(),
        };
        player.load_title(player.title_id, cx);
        player
    }

    /// The screen showing `state` with nothing playing, for looking at it in `shot`.
    pub fn preview(
        core: Arc<Core>,
        request: Request,
        state: PlayerState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut player = Self::blank(core, request, window, cx);
        player.state = Some(state);
        player.frame_shown = true;
        player
    }

    /// The colours of the title playing.
    pub fn palette(&self) -> &Palette {
        &self.request.palette
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(Closed);
    }

    /// Runs a call on the player in the core and shows what went wrong.
    fn run(
        &mut self,
        job: impl Future<Output = Result<(), AppError>> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let job = self.core.rt.spawn(job);
        cx.spawn(async move |player, cx| {
            if let Ok(Err(e)) = job.await {
                player
                    .update(cx, |p, cx| {
                        p.toast = Some((e.message, Instant::now()));
                        p.later(TOAST_SHOWN, cx);
                        cx.notify();
                    })
                    .ok();
            }
        })
        .detach();
    }

    fn control(&mut self, action: &'static str, value: Option<f64>, cx: &mut Context<Self>) {
        let player = self.core.player.clone();
        self.run(async move { player.control(action, value).await }, cx);
    }

    /// Plays an episode; a failure to start shows in the middle of the screen.
    fn play(&mut self, id: i64, episode: i64, dub: Option<String>, cx: &mut Context<Self>) {
        self.play_at(id, episode, dub, None, cx);
    }

    fn play_at(
        &mut self,
        id: i64,
        episode: i64,
        dub: Option<String>,
        start: Option<f64>,
        cx: &mut Context<Self>,
    ) {
        self.error = None;
        self.ended = None;
        self.frame_shown = false;
        let player = self.core.player.clone();
        let job = self
            .core
            .rt
            .spawn(async move { player.play(id, episode, dub, start).await });
        cx.spawn(async move |this, cx| {
            if let Ok(Err(e)) = job.await {
                this.update(cx, |p, cx| {
                    p.error = Some(e.message);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Redraws once `after` has passed, for things that go away by themselves.
    fn later(&mut self, after: Duration, cx: &mut Context<Self>) {
        self.timers.retain(|t| !t.is_ready());
        self.timers.push(cx.spawn(async move |player, cx| {
            cx.background_executor().timer(after).await;
            player.update(cx, |_, cx| cx.notify()).ok();
        }));
    }

    /// The mouse moved: the controls show for a while.
    fn poke(&mut self, cx: &mut Context<Self>) {
        let was_idle = !self.controls_shown();
        self.moved_at = Instant::now();
        self.later(HIDE_AFTER + Duration::from_millis(20), cx);
        if was_idle {
            cx.notify();
        }
    }

    /// The controls stay while paused, loading, or with a panel or menu open.
    fn controls_shown(&self) -> bool {
        let keep = match &self.state {
            None => true,
            Some(s) => s.paused || s.buffering,
        } || self.panel.is_some()
            || self.quality_menu.is_open()
            || self.upscale_menu.is_open()
            || self.sheet_dub_menu.is_open();
        keep || self.moved_at.elapsed() < HIDE_AFTER
    }

    fn load_title(&mut self, id: i64, cx: &mut Context<Self>) {
        self.title_id = id;
        self.sheet_part = id;
        let core = self.core.clone();
        let job = core.rt.spawn({
            let core = core.clone();
            async move {
                let anime = core.catalog.get(id).await.ok();
                let details = core.details.get(id).await.ok();
                let sources = core.resolver.title_sources(id).await.ok();
                (anime, details, sources)
            }
        });
        cx.spawn(async move |player, cx| {
            if let Ok((anime, details, sources)) = job.await {
                player
                    .update(cx, |p, cx| {
                        if p.title_id == id {
                            (p.anime, p.details, p.sources) = (anime, details, sources);
                            cx.notify();
                        }
                    })
                    .ok();
            }
        })
        .detach();
    }

    /// What the episodes sheet needs about a season, fetched the first time it is shown.
    fn load_part(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.parts.contains_key(&id) {
            return;
        }
        self.parts.insert(id, PartData::default());
        let core = self.core.clone();
        let job = core.rt.spawn({
            let core = core.clone();
            async move {
                let anime = core.catalog.get(id).await.ok();
                let card = anime.clone().map(|a| feeds::card(&core.posters, a));
                let sources = core.resolver.title_sources(id).await.ok();
                let progress = core.library.progress(id).unwrap_or_default();
                let cont = anime
                    .as_ref()
                    .and_then(|a| feeds::continue_for(a, &progress));
                let chosen_dub = core
                    .library
                    .get_entry(id)
                    .ok()
                    .flatten()
                    .and_then(|e| e.chosen_dub_key);
                PartData {
                    card,
                    sources,
                    progress,
                    cont,
                    chosen_dub,
                    marks: Vec::new(),
                }
            }
        });
        // Fillers come from Jikan, which can be slow: the episodes do not wait for them.
        let marks = core.rt.spawn({
            let core = core.clone();
            async move {
                let ongoing = core
                    .catalog
                    .cached_many(&[id])
                    .ok()
                    .and_then(|a| a.into_iter().next())
                    .is_none_or(|a| a.status.as_deref() != Some("released"));
                core.jikan.marks(id, ongoing).await
            }
        });
        cx.spawn(async move |player, cx| {
            if let Ok(data) = job.await {
                player
                    .update(cx, |p, cx| {
                        let marks = p.parts.remove(&id).map(|d| d.marks).unwrap_or_default();
                        p.parts.insert(id, PartData { marks, ..data });
                        cx.notify();
                    })
                    .ok();
            }
            if let Ok(marks) = marks.await {
                player
                    .update(cx, |p, cx| {
                        p.parts.entry(id).or_default().marks = marks;
                        cx.notify();
                    })
                    .ok();
            }
        })
        .detach();
    }

    /// What the core's player says.
    pub fn on_event(&mut self, event: PlayerEvent, cx: &mut Context<Self>) {
        match event {
            PlayerEvent::State(s) => {
                if s.duration > 0. && !s.buffering {
                    self.frame_shown = true;
                }
                if self.state.as_ref().is_some_and(|old| {
                    (old.shikimori_id, old.episode) != (s.shikimori_id, s.episode)
                }) {
                    self.thumbs.clear();
                    self.skipped = None;
                    // The season's episodes sheet shows the new progress next time.
                    self.parts.clear();
                }
                if s.shikimori_id != self.title_id {
                    self.load_title(s.shikimori_id, cx);
                }
                self.state = Some(s);
            }
            PlayerEvent::Thumbs(t) => {
                if self
                    .state
                    .as_ref()
                    .is_some_and(|s| (s.shikimori_id, s.episode) == (t.shikimori_id, t.episode))
                {
                    self.thumbs = t.items;
                }
            }
            PlayerEvent::Progress(_) => return,
            PlayerEvent::Error { message, .. } => self.error = Some(message),
            PlayerEvent::Stopped => self.state = None,
            PlayerEvent::Skipped { part, from } => {
                self.skipped = Some((part, from, Instant::now()));
                self.later(SKIPPED_SHOWN, cx);
            }
            PlayerEvent::Finished {
                shikimori_id,
                episode,
            } => self.ended = Some((shikimori_id, episode)),
        }
        cx.notify();
    }

    fn set_panel(&mut self, panel: Option<Panel>, cx: &mut Context<Self>) {
        self.panel = panel;
        if panel == Some(Panel::Episodes) {
            self.sheet_part = self.title_id;
            self.load_part(self.title_id, cx);
        }
        cx.notify();
    }

    fn toggle_panel(&mut self, panel: Panel, cx: &mut Context<Self>) {
        let next = (self.panel != Some(panel)).then_some(panel);
        self.set_panel(next, cx);
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.poke(cx);
        let m = e.keystroke.modifiers;
        if m.alt || m.control || m.platform {
            return;
        }
        let (volume, delay) = self
            .state
            .as_ref()
            .map_or((100., 0.), |s| (s.volume, s.audio_delay));
        match latin(e.keystroke.key.as_str()) {
            "space" | "k" => self.control("toggle_pause", None, cx),
            "left" => self.control("seek_by", Some(-10.), cx),
            "right" => self.control("seek_by", Some(10.), cx),
            "up" => self.control("volume", Some(volume + 5.), cx),
            "down" => self.control("volume", Some(volume - 5.), cx),
            "f" => window.toggle_fullscreen(),
            "m" => self.control("toggle_mute", None, cx),
            "n" => self.control("next", None, cx),
            "p" => self.control("prev", None, cx),
            "u" => self.control("toggle_upscale", None, cx),
            // Held: the original picture, to compare.
            "c" if !e.is_held => self.control("upscale_compare", Some(1.), cx),
            "z" => self.control("toggle_fill", None, cx),
            "[" => self.control(
                "audio_delay",
                Some(player_panels::nudge_delay(delay, -1.)),
                cx,
            ),
            "]" => self.control(
                "audio_delay",
                Some(player_panels::nudge_delay(delay, 1.)),
                cx,
            ),
            "escape" => {
                if self.quality_menu.is_open() {
                    return;
                }
                if self.panel.is_some() {
                    self.set_panel(None, cx);
                } else if window.is_fullscreen() {
                    window.toggle_fullscreen();
                } else {
                    self.leave(window, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.is_fullscreen() {
            window.toggle_fullscreen();
        }
        cx.emit(Closed);
    }

    /// One click pauses, a double click goes fullscreen; a click with a panel open closes it.
    fn on_surface_click(&mut self, e: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel.is_some() {
            self.set_panel(None, cx);
            return;
        }
        if e.click_count() >= 2 {
            self.pending_click = None;
            window.toggle_fullscreen();
            return;
        }
        self.pending_click = Some(cx.spawn(async move |player, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(220))
                .await;
            player
                .update(cx, |p, cx| p.control("toggle_pause", None, cx))
                .ok();
        }));
    }
}

/// Stops a press from reaching the screen under it, where it would pause the video.
fn solid<E: InteractiveElement>(el: E) -> E {
    el.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// A round-cornered button in the control bar (`CtrlButton`).
fn ctrl(
    id: impl Into<gpui::ElementId>,
    glyph: &str,
    size: f32,
    active: bool,
    accent: Hsla,
) -> Stateful<Div> {
    div()
        .id(id)
        .cursor_pointer()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(44.))
        .min_w(px(44.))
        .rounded(theme::R_M)
        .map(|d| {
            if active {
                d.text_color(accent).bg(accent.opacity(0.16))
            } else {
                d.text_color(rgba(0xffffffe0))
                    .hover(|s| s.text_color(white()).bg(rgba(0xffffff1f)))
            }
        })
        .child(icon(glyph, size).text_color(if active {
            accent
        } else {
            rgba(0xffffffe0).into()
        }))
}

/// A glass pill floating over the video; `accent` makes it the main one.
fn pill(
    id: impl Into<gpui::ElementId>,
    glyph: Option<&str>,
    label: impl Into<SharedString>,
    accent: Option<Hsla>,
) -> Stateful<Div> {
    let fg: Hsla = if accent.is_some() {
        rgb(theme::ACCENT_CONTRAST).into()
    } else {
        white()
    };
    div()
        .id(id)
        .cursor_pointer()
        .flex()
        .items_center()
        .gap(px(8.))
        .pl(px(16.))
        .pr(px(20.))
        .py(px(12.))
        .rounded_full()
        .font_weight(FontWeight::BOLD)
        .text_color(fg)
        .map(|d| match accent {
            Some(a) => d
                .bg(a)
                .shadow(vec![BoxShadow {
                    color: a.opacity(0.45),
                    offset: point(px(0.), px(10.)),
                    blur_radius: px(40.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .hover(move |s| s.bg(theme::mix(a, white(), 0.12))),
            None => d
                .bg(rgba(0xffffff24))
                .border_1()
                .border_color(rgba(0xffffff29))
                .hover(|s| s.bg(rgba(0xffffff3d))),
        })
        .children(glyph.map(|g| icon(g, 18.).text_color(fg)))
        .child(label.into())
}

/// Slides a floating pill in from the right, once, when it appears.
fn slide_in(el: impl IntoElement, id: impl Into<gpui::ElementId>) -> impl IntoElement {
    div().child(el).with_animation(
        id,
        Animation::new(Duration::from_millis(420)).with_easing(|t| SLIDE.at(t)),
        |d, t| {
            d.opacity(t.clamp(0., 1.))
                .relative()
                .left(px(28. * (1. - t)))
        },
    )
}

/// The video's frame is drawn over and over while the mouse drags: moves and the release are
/// followed over the whole window, as a pointer capture does.
fn follow_drag(
    dragging: impl Fn() -> bool + 'static,
    on_move: impl Fn(Pixels, &mut Window, &mut gpui::App) + 'static,
    on_up: impl Fn(Pixels, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |_, _, window, _| {
            if !dragging() {
                return;
            }
            let on_move = Rc::new(on_move);
            window.on_mouse_event(move |e: &MouseMoveEvent, _, window, cx| {
                if e.pressed_button == Some(MouseButton::Left) {
                    on_move(e.position.x, window, cx);
                }
            });
            let on_up = Rc::new(on_up);
            window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
                if phase.capture() && e.button == MouseButton::Left {
                    on_up(e.position.x, window, cx);
                }
            });
        },
    )
    .absolute()
    .size_0()
}

impl Player {
    fn seek_bar(
        &mut self,
        s: &PlayerState,
        accent: Hsla,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let duration = s.duration;
        let bounds = self.seek_bounds.clone();
        let at = move |x: Pixels| {
            let b = bounds.get();
            if b.size.width <= px(0.) {
                return 0.;
            }
            f64::from(((x - b.left()) / b.size.width).clamp(0., 1.)) * duration
        };
        let pending = self.seek_to.get().filter(|(to, at)| {
            at.elapsed() < Duration::from_secs(3) && (s.position - to).abs() > 1.
        });
        if pending.is_none() {
            self.seek_to.set(None);
        }
        let shown = self
            .seek_drag
            .get()
            .or(pending.map(|(to, _)| to))
            .unwrap_or(s.position);
        let pct = |t: f64| {
            if duration > 0. {
                relative((t / duration).clamp(0., 1.) as f32)
            } else {
                relative(0.)
            }
        };
        let active = self.seek_hover.is_some() || self.seek_drag.get().is_some();
        motion::set("seek-grow", active, theme::T_FAST, EASE, cx);
        let grow = motion::get("seek-grow", window, cx);
        let height = 4. + 3. * grow;
        let marks = [s.opening, s.ending].into_iter().flatten();

        let tip = self.seek_hover.map(|t| {
            let b = self.seek_bounds.get();
            let x = if duration > 0. {
                b.size.width * (t / duration) as f32
            } else {
                px(0.)
            };
            let x = x.clamp(
                px(96.).min(b.size.width / 2.),
                (b.size.width - px(96.)).max(b.size.width / 2.),
            );
            let frame = thumb_at(&self.thumbs, t).map(|th| {
                img(std::path::PathBuf::from(&th.path))
                    .w(px(176.))
                    .h(px(99.))
                    .rounded(px(8.))
                    .object_fit(ObjectFit::Cover)
            });
            div()
                .absolute()
                .bottom(px(26.))
                .left(x - px(100.))
                .w(px(200.))
                .flex()
                .justify_center()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(6.))
                        .px(px(9.))
                        .py(px(4.))
                        .rounded(px(8.))
                        .bg(rgba(0x0a0a0ee0))
                        .text_size(px(12.))
                        .font_weight(FontWeight::BOLD)
                        .children(frame)
                        .child(clock(t)),
                )
        });

        let (drag, hover_bounds) = (self.seek_drag.clone(), self.seek_bounds.clone());
        let on_down_at = at.clone();
        let (check_drag, move_drag, up_drag) = (
            self.seek_drag.clone(),
            self.seek_drag.clone(),
            self.seek_drag.clone(),
        );
        let (move_at, up_at) = (at.clone(), at.clone());
        let this = cx.entity().downgrade();
        let this_up = this.clone();
        let seek_to = self.seek_to.clone();
        div()
            .id("seek")
            .relative()
            .h(px(22.))
            .flex()
            .items_center()
            .cursor_pointer()
            .on_mouse_move(cx.listener(move |p, e: &MouseMoveEvent, _, cx| {
                p.seek_hover = Some(at(e.position.x));
                cx.notify();
            }))
            .on_hover(cx.listener(|p, hovered: &bool, _, cx| {
                if !hovered {
                    p.seek_hover = None;
                    cx.notify();
                }
            }))
            .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                drag.set(Some(on_down_at(e.position.x)));
                window.refresh();
            })
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(height))
                    .rounded_full()
                    .bg(rgba(0xffffff2e))
                    .children(marks.map(|(a, b)| {
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(pct(a))
                            .w(relative(((b - a) / duration.max(1.)).clamp(0., 1.) as f32))
                            .bg(rgba(0xffffff29))
                    }))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left_0()
                            .w(pct(s.cached_until))
                            .rounded_full()
                            .bg(rgba(0xffffff38)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left_0()
                            .w(pct(shown))
                            .rounded_full()
                            .bg(accent)
                            .shadow(vec![BoxShadow {
                                color: accent.opacity(0.7),
                                offset: point(px(0.), px(0.)),
                                blur_radius: px(14.),
                                spread_radius: px(0.),
                                inset: false,
                            }]),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(height / 2. - 8. * grow))
                            .left(pct(shown))
                            .ml(px(-8. * grow))
                            .size(px(16. * grow))
                            .rounded_full()
                            .bg(white())
                            .shadow(vec![BoxShadow {
                                color: accent.opacity(0.45),
                                offset: point(px(0.), px(0.)),
                                blur_radius: px(0.),
                                spread_radius: px(5. * grow),
                                inset: false,
                            }]),
                    ),
            )
            .children(tip)
            .child(
                canvas(move |b, _, _| hover_bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(follow_drag(
                move || check_drag.get().is_some(),
                move |x, window, _| {
                    move_drag.set(Some(move_at(x)));
                    window.refresh();
                },
                move |x, window, cx| {
                    if up_drag.take().is_some() {
                        let t = up_at(x);
                        seek_to.set(Some((t, Instant::now())));
                        this_up
                            .update(cx, |p, cx| p.control("seek", Some(t), cx))
                            .ok();
                        window.refresh();
                    }
                },
            ))
    }

    /// The mute button and, on hover, the volume slider sliding out of it.
    fn volume(
        &mut self,
        s: &PlayerState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        // Up to mpv's own limit: a quietly mixed dub can be lifted past 100%.
        let level = if s.muted {
            0.
        } else {
            s.volume.min(MAX_VOLUME) / MAX_VOLUME
        } as f32;
        let glyph = if s.muted || s.volume <= 0. {
            "mute"
        } else if s.volume < 50. {
            "volumeLow"
        } else {
            "volume"
        };
        let open = motion::get("volume-open", window, cx);
        let bounds = self.volume_bounds.clone();
        let at = move |x: Pixels| {
            let b = bounds.get();
            if b.size.width <= px(0.) {
                return None;
            }
            Some((f64::from((x - b.left()) / b.size.width).clamp(0., 1.) * MAX_VOLUME).round())
        };
        let (down_at, move_at) = (at.clone(), at.clone());
        let (dragging, drag_check, drag_end) = (
            self.volume_drag.clone(),
            self.volume_drag.clone(),
            self.volume_drag.clone(),
        );
        let this = cx.entity().downgrade();
        let this_move = this.clone();
        let hold = self.volume_drag.get();
        let track_bounds = self.volume_bounds.clone();
        div()
            .id("volume")
            .flex()
            .items_center()
            .on_hover(move |hovered, window, cx| {
                motion::set("volume-open", *hovered || hold, theme::T_MED, EASE, cx);
                window.refresh();
            })
            .child(
                ctrl("mute", glyph, 24., false, white())
                    .on_click(cx.listener(|p, _, _, cx| p.control("toggle_mute", None, cx))),
            )
            .child(
                div()
                    .id("volume-track")
                    .relative()
                    .w(px(92. * open))
                    .ml(px(4. * open))
                    .mr(px(12. * open))
                    .h(px(18.))
                    .flex()
                    .items_center()
                    .opacity(open.clamp(0., 1.))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        if let Some(v) = down_at(e.position.x) {
                            dragging.set(true);
                            this.update(cx, |p, cx| p.control("volume", Some(v), cx))
                                .ok();
                        }
                        window.refresh();
                    })
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h(px(4.))
                            .rounded_full()
                            .bg(rgba(0xffffff33))
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .bottom_0()
                                    .left_0()
                                    .w(relative(level))
                                    .rounded_full()
                                    .bg(white()),
                            )
                            // Where the source's own loudness ends and the boost begins.
                            .child(
                                div()
                                    .absolute()
                                    .top(px(-2.))
                                    .left(relative((100. / MAX_VOLUME) as f32))
                                    .w(px(2.))
                                    .h(px(8.))
                                    .rounded_full()
                                    .bg(rgba(0xffffff80)),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top(px(-5.))
                                    .left(relative(level))
                                    .ml(px(-7.))
                                    .size(px(14.))
                                    .rounded_full()
                                    .bg(white()),
                            ),
                    )
                    .child(
                        canvas(move |b, _, _| track_bounds.set(b), |_, _, _, _| {})
                            .absolute()
                            .size_full(),
                    )
                    .child(follow_drag(
                        move || drag_check.get(),
                        move |x, _, cx| {
                            if let Some(v) = move_at(x) {
                                this_move
                                    .update(cx, |p, cx| p.control("volume", Some(v), cx))
                                    .ok();
                            }
                        },
                        move |_, window, _| {
                            drag_end.set(false);
                            window.refresh();
                        },
                    )),
            )
    }

    fn quality(
        &mut self,
        s: &PlayerState,
        accent: Hsla,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let menu = self.quality_menu.clone();
        let auto = s.quality_cap.is_none();
        let mut items: Vec<(u32, String, Option<String>, bool)> = vec![(
            0,
            "Авто".into(),
            s.qualities.first().map(|q| format!("{q}p")),
            auto,
        )];
        items.extend(
            s.qualities
                .iter()
                .map(|q| (*q, format!("{q}p"), None, !auto && s.quality == Some(*q))),
        );
        let panel =
            div()
                .min_w(px(180.))
                .flex()
                .flex_col()
                .child(
                    div()
                        .px(px(12.))
                        .py(px(6.))
                        .text_size(px(12.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgba(0xffffff8c))
                        .child("Качество"),
                )
                .children(items.into_iter().map(|(value, label, hint, selected)| {
                    let close = menu.clone();
                    div()
                        .id(("quality", value as u64))
                        .cursor_pointer()
                        .flex()
                        .justify_between()
                        .items_center()
                        .gap(px(16.))
                        .px(px(12.))
                        .py(px(9.))
                        .rounded(px(9.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(if selected {
                            accent
                        } else {
                            rgb(theme::TEXT).into()
                        })
                        .hover(|st| st.bg(theme::glass_strong()))
                        .on_click(cx.listener(move |p, _, _, cx| {
                            close.close();
                            p.control("quality", Some(value as f64), cx);
                        }))
                        .child(
                            div().flex().gap(px(8.)).child(label).children(
                                hint.map(|h| div().text_color(rgb(theme::TEXT_3)).child(h)),
                            ),
                        )
                        .when(selected, |d| d.child(icon("check", 16.).text_color(accent)))
                }));
        let toggle = self.quality_menu.clone();
        div()
            .relative()
            .child(
                ctrl(
                    "quality",
                    "quality",
                    22.,
                    self.quality_menu.is_open(),
                    accent,
                )
                .px(px(10.))
                .gap(px(8.))
                .text_size(px(13.))
                .font_weight(FontWeight::BOLD)
                .children(s.quality.map(|q| format!("{q}p")))
                .on_click(cx.listener(move |_, e: &ClickEvent, _, cx| {
                    toggle.toggle(e);
                    cx.notify();
                })),
            )
            .children(popover_up(
                &self.quality_menu,
                "quality-menu",
                panel,
                window,
                cx,
            ))
    }

    /// The sparkle: the upscaler's panel above it.
    fn upscale(
        &mut self,
        s: &PlayerState,
        accent: Hsla,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let me = cx.entity().downgrade();
        let on_control: player_panels::OnControl = Rc::new(move |action, value, _, cx| {
            me.update(cx, |p, cx| p.control(action, Some(value), cx))
                .ok();
        });
        let panel = player_panels::upscale_panel(s, accent, on_control, window, cx);
        let toggle = self.upscale_menu.clone();
        div()
            .relative()
            .child(
                ctrl(
                    "upscale",
                    "sparkle",
                    24.,
                    s.upscale || self.upscale_menu.is_open(),
                    accent,
                )
                .on_click(cx.listener(move |_, e: &ClickEvent, _, cx| {
                    toggle.toggle(e);
                    cx.notify();
                })),
            )
            .children(popover_up(
                &self.upscale_menu,
                "upscale-menu",
                panel,
                window,
                cx,
            ))
    }

    fn bottom(
        &mut self,
        s: &PlayerState,
        accent: Hsla,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let seek = self.seek_bar(s, accent, window, cx);
        let volume = self.volume(s, window, cx);
        let quality = (s.qualities.len() > 1).then(|| self.quality(s, accent, window, cx));
        let fullscreen = window.is_fullscreen();
        let left = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .min_w(px(0.))
            .child(
                ctrl(
                    "pause",
                    if s.paused { "play" } else { "pause" },
                    28.,
                    false,
                    accent,
                )
                .on_click(cx.listener(|p, _, _, cx| p.control("toggle_pause", None, cx))),
            )
            .child(
                ctrl("rewind", "rewind", 24., false, accent)
                    .on_click(cx.listener(|p, _, _, cx| p.control("seek_by", Some(-10.), cx))),
            )
            .child(
                ctrl("forward", "forward", 24., false, accent)
                    .on_click(cx.listener(|p, _, _, cx| p.control("seek_by", Some(10.), cx))),
            )
            .when(s.next.is_some(), |d| {
                d.child(
                    ctrl("next", "next", 24., false, accent)
                        .on_click(cx.listener(|p, _, _, cx| p.control("next", None, cx))),
                )
            })
            .child(volume)
            .child(
                div()
                    .ml(px(12.))
                    .flex()
                    .text_size(px(13.))
                    .font_weight(FontWeight::BOLD)
                    .child(clock(s.position))
                    .when(s.duration > 0., |d| {
                        d.child(
                            div()
                                .text_color(rgba(0xffffff8c))
                                .child(format!(" / {}", clock(s.duration))),
                        )
                    }),
            );
        let panel = self.panel;
        let right = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .child(self.upscale(s, accent, window, cx))
            .children(quality)
            .child(
                ctrl(
                    "settings",
                    "settings",
                    24.,
                    panel == Some(Panel::Settings),
                    accent,
                )
                .on_click(cx.listener(|p, _, _, cx| p.toggle_panel(Panel::Settings, cx))),
            )
            .child(
                ctrl("dubs", "voice", 22., panel == Some(Panel::Dubs), accent)
                    .max_w(px(220.))
                    .pl(px(10.))
                    .pr(px(14.))
                    .gap(px(8.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::BOLD)
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(s.dub_name.clone()),
                    )
                    .on_click(cx.listener(|p, _, _, cx| p.toggle_panel(Panel::Dubs, cx))),
            )
            .child(
                ctrl(
                    "fill",
                    if s.fill { "fit" } else { "fill" },
                    24.,
                    s.fill,
                    accent,
                )
                .on_click(cx.listener(|p, _, _, cx| p.control("toggle_fill", None, cx))),
            )
            .child(
                ctrl(
                    "episodes",
                    "episodes",
                    24.,
                    panel == Some(Panel::Episodes),
                    accent,
                )
                .on_click(cx.listener(|p, _, _, cx| p.toggle_panel(Panel::Episodes, cx))),
            )
            .child(
                ctrl(
                    "fullscreen",
                    if fullscreen {
                        "fullscreenExit"
                    } else {
                        "fullscreen"
                    },
                    24.,
                    false,
                    accent,
                )
                .on_click(|_, window, _| window.toggle_fullscreen()),
            );
        solid(
            div()
                .id("bottom")
                .flex()
                .flex_col()
                .gap(px(8.))
                .px(px(32.))
                .pb(px(24.))
                .child(seek)
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .gap(px(16.))
                        .child(left)
                        .child(right),
                ),
        )
    }

    fn top(&mut self, accent: Hsla, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let s = self.state.as_ref();
        let title = s.map_or(self.request.title.to_string(), |s| s.title.clone());
        let sub = s.map(|s| {
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .text_color(rgba(0xffffffb8))
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("Серия {}", s.episode))
                .child(s.dub_name.clone())
                .children(quality_badge(s.quality))
                .when((s.speed - 1.).abs() > 0.01, |d| {
                    d.child(format!("{}×", s.speed.to_string().replace('.', ",")))
                })
        });
        let _ = accent;
        solid(
            div()
                .id("top")
                .flex()
                .items_center()
                .gap(px(16.))
                .px(px(32.))
                .py(px(24.))
                .child(
                    div()
                        .id("back")
                        .cursor_pointer()
                        .flex_none()
                        .size(px(44.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(rgba(0xffffff1f))
                        .hover(|st| st.bg(rgba(0xffffff38)))
                        .on_click(cx.listener(|p, _, window, cx| p.leave(window, cx)))
                        .child(icon("back", 22.).text_color(white())),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(24.))
                                .font_weight(FontWeight::EXTRA_BOLD)
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(title),
                        )
                        .children(sub),
                ),
        )
    }

    /// Loading, the big play button, or a card about an error or the end.
    fn center(&mut self, accent: Hsla, cx: &mut Context<Self>) -> Option<AnyElement> {
        let card = |icon_name: &str, color: Hsla| {
            solid(
                div()
                    .id("card")
                    .max_w(px(460.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(12.))
                    .px(px(32.))
                    .py(px(24.))
                    .rounded(theme::R_L)
                    .bg(rgba(0x12121adb))
                    .text_color(rgb(theme::TEXT))
                    .child(icon(icon_name, 30.).text_color(color)),
            )
        };
        let back = pill("card-back", None, "Вернуться к тайтлу", None)
            .on_click(cx.listener(|p, _, window, cx| p.leave(window, cx)));
        if let Some(e) = &self.error {
            return Some(
                card("warn", rgb(0xf0c867).into())
                    .child(div().text_center().child(e.clone()))
                    .child(back)
                    .into_any_element(),
            );
        }
        let s = self.state.as_ref()?;
        let ended = self.ended.is_some_and(|(id, ep)| {
            (id, ep) == (s.shikimori_id, s.episode) && s.duration - s.position < 10.
        });
        if ended {
            let released = self
                .anime
                .as_ref()
                .is_some_and(|a| a.status.as_deref() == Some("released"));
            let next = self
                .details
                .as_ref()
                .and_then(|d| next_season(&d.parts, s.shikimori_id));
            let next_label = self
                .anime
                .as_ref()
                .and_then(|a| next_episode_label(a.next_episode_at.as_deref()));
            let mut c = card("check", accent);
            if released {
                c = c.child(
                    div()
                        .text_size(px(18.))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .child(if next.is_some() {
                            "Сезон просмотрен"
                        } else {
                            "Сериал просмотрен"
                        }),
                );
            } else {
                c = c
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child("Это последняя вышедшая серия"),
                    )
                    .children(next_label);
            }
            if let Some((id, label)) = next {
                c = c.child(
                    pill(
                        "next-season",
                        None,
                        format!("Смотреть {label}"),
                        Some(accent),
                    )
                    .on_click(cx.listener(move |p, _, _, cx| p.play(id, 1, None, cx))),
                );
            }
            return Some(c.child(back).into_any_element());
        }
        if s.buffering || !self.frame_shown {
            return Some(spinner(accent).into_any_element());
        }
        if s.paused {
            return Some(
                solid(
                    div()
                        .id("big-play")
                        .cursor_pointer()
                        .size(px(92.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(accent)
                        .hover(move |st| st.bg(theme::mix(accent, white(), 0.1)))
                        .shadow(vec![
                            BoxShadow {
                                color: accent.opacity(0.22),
                                offset: point(px(0.), px(0.)),
                                blur_radius: px(0.),
                                spread_radius: px(10.),
                                inset: false,
                            },
                            BoxShadow {
                                color: rgba(0x00000073).into(),
                                offset: point(px(0.), px(20.)),
                                blur_radius: px(60.),
                                spread_radius: px(0.),
                                inset: false,
                            },
                        ])
                        .on_click(cx.listener(|p, _, _, cx| p.control("toggle_pause", None, cx)))
                        .child(
                            div()
                                .ml(px(5.))
                                .child(icon("play", 40.).text_color(rgb(theme::ACCENT_CONTRAST))),
                        ),
                )
                .with_animation(
                    "big-play-in",
                    Animation::new(Duration::from_millis(360)).with_easing(|t| SLIDE.at(t)),
                    |d, t| d.opacity(t.clamp(0., 1.)),
                )
                .into_any_element(),
            );
        }
        None
    }

    fn floating(&mut self, s: &PlayerState, accent: Hsla, cx: &mut Context<Self>) -> Div {
        let (skip_opening, show_next) = overlays(s);
        let skipped = self
            .skipped
            .filter(|(_, _, at)| at.elapsed() < SKIPPED_SHOWN);
        div()
            .absolute()
            .right(px(32.))
            .bottom(px(124.))
            .flex()
            .flex_col()
            .items_end()
            .gap(px(8.))
            .children(skipped.map(|(part, from, _)| {
                slide_in(
                    solid(
                        pill(
                            "bring-back",
                            Some("rewind"),
                            if part == "opening" {
                                "Вернуть опенинг"
                            } else {
                                "Вернуть эндинг"
                            },
                            None,
                        )
                        .on_click(cx.listener(move |p, _, _, cx| {
                            p.skipped = None;
                            p.control("seek", Some(from), cx);
                        })),
                    ),
                    "bring-back-in",
                )
            }))
            .when(skip_opening, |d| {
                d.child(slide_in(
                    solid(
                        pill("skip-opening", Some("skip"), "Пропустить опенинг", None).on_click(
                            cx.listener(|p, _, _, cx| p.control("skip_opening", None, cx)),
                        ),
                    ),
                    "skip-opening-in",
                ))
            })
            .when(show_next, |d| {
                d.child(slide_in(
                    solid(
                        pill(
                            "next-episode",
                            Some("next"),
                            "Следующая серия",
                            Some(accent),
                        )
                        .on_click(cx.listener(|p, _, _, cx| p.control("next", None, cx))),
                    ),
                    "next-episode-in",
                ))
            })
    }

    fn sheet(
        &mut self,
        panel: Panel,
        s: &PlayerState,
        accent: Hsla,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let body: AnyElement = match panel {
            Panel::Settings => {
                let this = cx.entity().downgrade();
                let on_change: player_panels::OnChange = Rc::new(move |change, _, cx| {
                    this.update(cx, |p, cx| match change {
                        Change::Speed(v) => p.control("speed", Some(v), cx),
                        Change::Skip("opening") => p.control("toggle_skip_opening", None, cx),
                        Change::Skip("ending") => p.control("toggle_skip_ending", None, cx),
                        Change::Skip(_) => p.control("toggle_skip_fillers", None, cx),
                        Change::Effects(fx) => {
                            let player = p.core.player.clone();
                            p.run(async move { player.set_audio(fx).await }, cx);
                        }
                        Change::Delay(d) => p.control("audio_delay", Some(d), cx),
                        Change::Track(id) => p.control("audio_track", Some(id as f64), cx),
                    })
                    .ok();
                });
                player_panels::settings(s, accent, on_change, window, cx).into_any_element()
            }
            Panel::Dubs => {
                let this = cx.entity().downgrade();
                let current = s.dub_key.clone();
                player_panels::dubs(
                    self.sources.as_ref(),
                    s,
                    accent,
                    Rc::new(move |key, _, cx| {
                        this.update(cx, |p, cx| {
                            p.set_panel(None, cx);
                            if key != current {
                                let player = p.core.player.clone();
                                p.run(async move { player.set_dub(key).await }, cx);
                            }
                        })
                        .ok();
                    }),
                    {
                        let (player, rt) = (self.core.player.clone(), self.core.rt.clone());
                        Rc::new(move |key, _| {
                            let player = player.clone();
                            rt.spawn(async move { player.warm_dub(key) });
                        })
                    },
                )
                .into_any_element()
            }
            Panel::Episodes => {
                let parts = self.details.as_ref().map_or(&[][..], |d| &d.parts[..]);
                let shown = self.sheet_part;
                let (on_part, on_dub, on_play) = {
                    let (a, b, c) = (
                        cx.entity().downgrade(),
                        cx.entity().downgrade(),
                        cx.entity().downgrade(),
                    );
                    (
                        Rc::new(move |id: i64, _: &mut Window, cx: &mut gpui::App| {
                            a.update(cx, |p, cx| {
                                p.sheet_part = id;
                                p.load_part(id, cx);
                                cx.notify();
                            })
                            .ok();
                        }) as Rc<dyn Fn(i64, &mut Window, &mut gpui::App)>,
                        Rc::new(move |key: String, _: &mut Window, cx: &mut gpui::App| {
                            b.update(cx, |p, cx| {
                                p.sheet_dub.insert(p.sheet_part, key);
                                cx.notify();
                            })
                            .ok();
                        })
                            as Rc<dyn Fn(String, &mut Window, &mut gpui::App)>,
                        Rc::new(
                            move |id: i64,
                                  n: i64,
                                  dub: Option<String>,
                                  _: &mut Window,
                                  cx: &mut gpui::App| {
                                c.update(cx, |p, cx| {
                                    p.set_panel(None, cx);
                                    p.play(id, n, dub, cx);
                                })
                                .ok();
                            },
                        )
                            as Rc<dyn Fn(i64, i64, Option<String>, &mut Window, &mut gpui::App)>,
                    )
                };
                player_panels::episodes(
                    parts,
                    shown,
                    self.parts.get(&shown),
                    self.sheet_dub.get(&shown).map(String::as_str),
                    s,
                    accent,
                    // The sheet's padding and its 1px edge on both sides.
                    SHEET_W - 34.,
                    on_part,
                    on_dub,
                    &self.sheet_dub_menu,
                    on_play,
                    window,
                    cx,
                )
                .into_any_element()
            }
        };
        solid(
            div()
                .id("sheet")
                .absolute()
                .top(px(16.))
                .right(px(16.))
                .bottom(px(120.))
                .w(px(SHEET_W))
                .flex()
                .flex_col()
                .gap(px(12.))
                .p(px(16.))
                .rounded(theme::R_L)
                .bg(rgba(0x101018f0))
                .border_1()
                .border_color(rgba(0xffffff14))
                .shadow(vec![BoxShadow {
                    color: rgba(0x00000080).into(),
                    offset: point(px(0.), px(30.)),
                    blur_radius: px(80.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .text_color(rgb(theme::TEXT))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_size(px(18.))
                                .font_weight(FontWeight::EXTRA_BOLD)
                                .child(panel.title()),
                        )
                        .child(
                            ctrl("close-sheet", "close", 20., false, accent)
                                .on_click(cx.listener(|p, _, _, cx| p.set_panel(None, cx))),
                        ),
                )
                .child(
                    div()
                        .id("sheet-body")
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .child(body),
                ),
        )
        .with_animation(
            ("sheet-in", panel as u64),
            Animation::new(Duration::from_millis(380))
                .with_easing(|t| Curve(0.22, 1., 0.36, 1.).at(t)),
            |d, t| d.opacity(t.clamp(0., 1.)).right(px(16. - 40. * (1. - t))),
        )
    }
}

/// A ring with an arc in the accent going round (`.spinner`).
fn spinner(accent: Hsla) -> impl IntoElement {
    div()
        .relative()
        .size(px(64.))
        .child(
            svg()
                .path("icons/ring.svg")
                .absolute()
                .size_full()
                .text_color(rgba(0xffffff24)),
        )
        .child(
            svg()
                .path("icons/arc.svg")
                .absolute()
                .size_full()
                .text_color(accent)
                .with_animation(
                    "spin",
                    Animation::new(Duration::from_millis(900)).repeat(),
                    |svg, t| svg.with_transformation(Transformation::rotate(radians(2. * PI * t))),
                ),
        )
}

impl Render for Player {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = Accent::colors(cx).0;
        let shown = self.controls_shown();
        motion::set("chrome", shown, Duration::from_millis(320), EASE, cx);
        let chrome = motion::get("chrome", window, cx);
        let ended = self.ended.is_some_and(|(id, ep)| {
            self.state.as_ref().is_some_and(|s| {
                (id, ep) == (s.shikimori_id, s.episode) && s.duration - s.position < 10.
            })
        });
        // The window is see-through: without a frame under it the desktop would show.
        let cover = !self.frame_shown || self.state.is_none() || self.error.is_some() || ended;
        if self
            .toast
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= TOAST_SHOWN)
        {
            self.toast = None;
        }

        let state = self.state.clone();
        let top = self.top(accent, cx);
        let center = self.center(accent, cx);
        // A panel covers that corner: the pills would show through it.
        let floating = state
            .as_ref()
            .filter(|_| self.panel.is_none())
            .map(|s| self.floating(s, accent, cx));
        let bottom = state.as_ref().map(|s| self.bottom(s, accent, window, cx));
        let sheet = match (self.panel, &state) {
            (Some(panel), Some(s)) => Some(self.sheet(panel, s, accent, window, cx)),
            _ => None,
        };
        let toast = self.toast.as_ref().map(|(text, _)| {
            div()
                .absolute()
                .top(px(32.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(18.))
                        .py(px(10.))
                        .rounded_full()
                        .bg(rgba(0x12121ae6))
                        .text_color(rgb(theme::TEXT))
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(text.clone()),
                )
        });

        let fade = |height: f32, top: bool| {
            let dark = rgba(if top { 0x0000009e } else { 0x000000c7 });
            let clear = rgba(0x00000000);
            div()
                .absolute()
                .left_0()
                .right_0()
                .h(px(height))
                .map(|d| if top { d.top_0() } else { d.bottom_0() })
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(if top { dark } else { clear }, 0.),
                    linear_color_stop(if top { clear } else { dark }, 1.),
                ))
        };

        div()
            .id("player")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_key_up(cx.listener(|p, e: &KeyUpEvent, _, cx| {
                if latin(e.keystroke.key.as_str()) == "c" {
                    p.control("upscale_compare", Some(0.), cx);
                }
            }))
            .on_mouse_move(cx.listener(|p, _, _, cx| p.poke(cx)))
            .on_click(cx.listener(Self::on_surface_click))
            .relative()
            .size_full()
            .text_color(white())
            .when(cover, |d| d.bg(gpui::black()))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .opacity(chrome.clamp(0., 1.))
                    .child(fade(180., true))
                    .child(fade(240., false))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .flex()
                            .flex_col()
                            .child(div().relative().top(px(-12. * (1. - chrome))).child(top))
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .children(center),
                            )
                            .children(
                                bottom.map(|b| {
                                    div().relative().top(px(14. * (1. - chrome))).child(b)
                                }),
                            ),
                    )
                    .children(floating),
            )
            .children(sheet)
            .children(toast)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn russian_keys_mean_the_same() {
        assert_eq!(latin("а"), "f");
        assert_eq!(latin("х"), "[");
        assert_eq!(latin("space"), "space");
    }

    #[test]
    fn clock_reads_minutes_and_hours() {
        assert_eq!(clock(65.), "1:05");
        assert_eq!(clock(3723.), "1:02:03");
    }
}
