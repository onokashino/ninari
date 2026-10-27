//! A title's page: the banner and header with the facts, continue, the library status and the
//! user's score, then the episodes and the rest about the title.

use std::cell::Cell;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AnchoredPositionMode, Animation, AnimationExt, AnyElement, Bounds, BoxShadow, ClickEvent,
    Context, Div, Entity, EventEmitter, FontWeight, Hsla, IntoElement, ObjectFit, Pixels, Render,
    RenderImage, Task, Window, anchored, canvas, deferred, div, img, point, prelude::*, px,
    relative, rgb, rgba,
};
use image::{Frame, RgbaImage};
use ninari_core::catalog::Anime;
use ninari_core::catalog::details::{Part, TitleDetails};
use ninari_core::catalog::jikan::EpisodeMark;
use ninari_core::feeds::{self, AnimeCard, ContinueAt};
use ninari_core::library::{EpisodeProgress, LibraryEntry};
use ninari_core::sources::TitleSources;

use crate::OnClick;
use crate::assets::icon;
use crate::button::{Size, Variant, button};
use crate::cards::{CardText, poster_card, poster_frame};
use crate::catalog_page::Seed;
use crate::clock::next_episode_label;
use crate::core::Core;
use crate::data::{Load, fetch, title};
use crate::episode_grid::{chip, episode_grid};
use crate::episodes::episode_tiles;
use crate::facts::{Fact, Tone, time_left, title_facts};
use crate::format::library_status;
use crate::franchise::part_chips;
use crate::lazy_grid::{self, Lazy};
use crate::motion;
use crate::motion::rise;
use crate::nav::Route;
use crate::next_episode::{NextUp, OnPlay, next_card, next_up};
use crate::player::Request;
use crate::popover::{Menu, popover};
use crate::posters::poster;
use crate::rail;
use crate::scroll_row::Row;
use crate::season_cards::{OnPart, PartCard, season_cards};
use crate::season_switch::{Phase, phase};
use crate::select::{SelectOption, select};
use crate::source_fix::{FixEvent, SourceFix};
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, Accent};
use crate::title_about::{More, about, videos};

pub enum TitleEvent {
    Open(Route),
    Play(Request),
    /// The library changed: lists elsewhere need a reload.
    Changed,
    /// The catalog of a genre or a studio.
    Browse(Seed),
    /// Another part of the franchise is shown in place: history and Root follow.
    Season(i64),
}

pub struct Progress {
    pub entry: Option<LibraryEntry>,
    pub progress: Vec<EpisodeProgress>,
    pub cont: Option<ContinueAt>,
}

/// Shikimori's words for its scores, so a score reads the same here and there.
const SCORE_WORDS: [&str; 11] = [
    "",
    "Хуже некуда",
    "Ужасно",
    "Очень плохо",
    "Плохо",
    "Более-менее",
    "Нормально",
    "Хорошо",
    "Отлично",
    "Великолепно",
    "Эпик вин!",
];

/// The note is saved once typing pauses this long.
const NOTE_PAUSE: std::time::Duration = std::time::Duration::from_millis(800);

const STATUSES: [&str; 5] = ["watching", "planned", "completed", "on_hold", "dropped"];

/// A franchise part's card: id, catalog card, watched / episodes when in the library.
type PartRow = (i64, Option<AnimeCard>, Option<(i64, i64)>);

/// A switch between seasons in progress.
struct Switch {
    from: i64,
    /// 1. forward, -1. back.
    dir: f32,
    started: Instant,
    /// When the new season's card was in, after `started`.
    ready: Option<Duration>,
}

/// What the page shows for one part of the franchise. A part opened once stays here, so going
/// back to it is instant.
struct Season {
    card: Load<AnimeCard>,
    progress: Load<Progress>,
    sources: Load<TitleSources>,
    /// The wide AniList picture's file: decoded only to cut a view, which is dropped off screen.
    banner: Option<PathBuf>,
    /// The banner cut and faded for a box this many logical pixels wide.
    banner_view: Option<(u32, Arc<RenderImage>)>,
    banner_pending: Option<u32>,
    details: Load<TitleDetails>,
    marks: Vec<EpisodeMark>,
    /// The dub picked on this page, over the one the library remembers.
    picked_dub: Option<String>,
    /// The hundred of episodes on screen; the one to continue in when `None`.
    episode_block: Option<usize>,
    similar: Load<Vec<AnimeCard>>,
}

impl Season {
    fn new() -> Self {
        Self {
            card: Load::Pending,
            progress: Load::Pending,
            sources: Load::Pending,
            banner: None,
            banner_view: None,
            banner_pending: None,
            details: Load::Pending,
            marks: Vec::new(),
            picked_dub: None,
            episode_block: None,
            similar: Load::Pending,
        }
    }
}

pub struct Title {
    /// The part on screen.
    shown: i64,
    /// The part drawn this frame: the old one while a switch fades it out.
    drawn: i64,
    /// The part the page was opened with: its entrances play once, not on a season switch.
    opened: i64,
    switch: Option<Switch>,
    /// The franchise's parts, in its order, once any part's details are in.
    franchise: Vec<Part>,
    parts: Load<Vec<PartRow>>,
    /// Counts the switches, so the animations keyed with it play again.
    nonce: u64,
    core: Arc<Core>,
    seasons: HashMap<i64, Season>,
    /// Banners for other widths, to free from the GPU on the next frame.
    stale: Vec<Arc<RenderImage>>,
    /// The episodes, the description, the videos and similar titles: built near the screen only.
    blocks: [Lazy; 4],
    status_menu: Menu,
    rating_menu: Menu,
    dub_menu: Menu,
    similar_row: Row,
    video_rows: [Row; 2],
    /// "Ещё" under the facts is open, and the height of what it hides.
    about_open: bool,
    about_h: Rc<Cell<Pixels>>,
    /// Where the header's buttons were last drawn: the slim bar shows once they are gone.
    actions_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The score under the mouse in the rating panel.
    hovered_score: Option<i64>,
    /// The note beside the score, saved once typing pauses.
    note: Entity<TextInput>,
    note_save: Option<Task<()>>,
    /// "Не тот тайтл?": the sources' matches, while open.
    fix: Option<Entity<SourceFix>>,
}

impl EventEmitter<TitleEvent> for Title {}

impl Title {
    pub fn new(core: Arc<Core>, id: i64, cx: &mut Context<Self>) -> Self {
        let note = cx.new(|cx| {
            let mut input = TextInput::new(
                "Заметка для себя: что понравилось, с какой серии смотреть, кому посоветовать",
                cx,
            );
            input.compact = true;
            input.search = false;
            input
        });
        cx.subscribe(&note, |page, _, event: &InputEvent, cx| match event {
            InputEvent::Changed => {
                page.note_save = Some(cx.spawn(async move |page, cx| {
                    cx.background_executor().timer(NOTE_PAUSE).await;
                    page.update(cx, |page, cx| page.save_note(cx)).ok();
                }));
            }
            InputEvent::Submit => page.save_note(cx),
            InputEvent::Image(_) => {}
        })
        .detach();
        let mut page = Self {
            note,
            note_save: None,
            fix: None,
            shown: id,
            drawn: id,
            opened: id,
            switch: None,
            nonce: 0,
            franchise: Vec::new(),
            parts: Load::Pending,
            core,
            seasons: HashMap::from([(id, Season::new())]),
            stale: Vec::new(),
            blocks: Default::default(),
            status_menu: Menu::default(),
            rating_menu: Menu::default(),
            dub_menu: Menu::default(),
            similar_row: Row::default(),
            video_rows: Default::default(),
            actions_bounds: Rc::default(),
            about_open: false,
            about_h: Rc::default(),
            hovered_score: None,
        };
        page.load(id, cx);
        page
    }

    fn cur(&self) -> &Season {
        &self.seasons[&self.drawn]
    }

    fn season_mut(&mut self, id: i64) -> &mut Season {
        self.seasons.entry(id).or_insert_with(Season::new)
    }

    /// The part on screen.
    /// Frees every banner: the page is off screen. They are cut again from the file when shown.
    pub fn release(&mut self, window: &mut Window) {
        self.drop_banners(window, |_| false);
    }

    fn drop_banners(&mut self, window: &mut Window, keep: impl Fn(i64) -> bool) {
        for (&id, season) in &mut self.seasons {
            if !keep(id)
                && let Some((_, view)) = season.banner_view.take()
            {
                window.drop_image(view).ok();
            }
        }
    }

    pub fn shown(&self) -> i64 {
        self.shown
    }

    /// Shows another part of the franchise in place, sliding towards it.
    pub fn show(&mut self, id: i64, cx: &mut Context<Self>) {
        use crate::season_switch::{OUT_MS, direction};
        if id == self.shown {
            return;
        }
        let order: Vec<i64> = self.franchise.iter().map(|p| p.id).collect();
        let dir = direction(&order, self.shown, id);
        let out = Duration::from_millis(OUT_MS);
        // A switch during a switch starts from what is on screen, already faded out.
        let started = match &self.switch {
            Some(s) if s.started.elapsed() < out => s.started,
            Some(_) => Instant::now() - out,
            None => Instant::now(),
        };
        self.switch = Some(Switch {
            from: self.drawn,
            dir,
            started,
            ready: None,
        });
        self.nonce += 1;
        self.shown = id;
        if let Entry::Vacant(slot) = self.seasons.entry(id) {
            slot.insert(Season::new());
            self.load(id, cx);
        }
        self.status_menu.close();
        self.rating_menu.close();
        self.dub_menu.close();
        self.fix = None;
        self.about_open = false;
        cx.notify();
    }

    fn load(&mut self, id: i64, cx: &mut Context<Self>) {
        fetch(
            cx,
            self.core.run(move |c| async move {
                let anime = c.catalog.get(id).await?;
                Ok(feeds::card(&c.posters, anime))
            }),
            move |page, card| page.season_mut(id).card = card,
        );
        self.load_progress(id, cx);
        self.load_sources(id, cx);
        fetch(
            cx,
            self.core.run(move |c| async move {
                let list = c.catalog.similar(id, 24).await?;
                Ok(list
                    .into_iter()
                    .map(|a| feeds::card(&c.posters, a))
                    .collect())
            }),
            move |page, similar| page.season_mut(id).similar = similar,
        );
        fetch(
            cx,
            self.core.run(move |c| async move {
                let anime = c.catalog.get(id).await?;
                c.posters.banner(&anime).await
            }),
            move |page, banner| {
                if let Load::Ready(b) = banner {
                    page.season_mut(id).banner = b;
                }
            },
        );
        fetch(
            cx,
            self.core
                .run(move |c| async move { c.details.get(id).await }),
            move |page, details| page.season_mut(id).details = details,
        );
        fetch(
            cx,
            self.core.run(move |c| async move {
                let ongoing = c
                    .catalog
                    .cached_many(&[id])?
                    .first()
                    .is_none_or(|a| a.status.as_deref() != Some("released"));
                Ok(c.jikan.marks(id, ongoing).await)
            }),
            move |page, marks| {
                if let Load::Ready(m) = marks {
                    page.season_mut(id).marks = m;
                }
            },
        );
    }

    fn load_sources(&mut self, id: i64, cx: &mut Context<Self>) {
        fetch(
            cx,
            self.core
                .run(move |c| async move { c.resolver.title_sources(id).await }),
            move |page, sources| page.season_mut(id).sources = sources,
        );
    }

    fn pick_dub(&mut self, key: String, cx: &mut Context<Self>) {
        let id = self.shown;
        self.season_mut(id).picked_dub = Some(key.clone());
        let job = self
            .core
            .run(move |c| async move { c.library.set_dub(id, &key) });
        cx.background_spawn(async move {
            if let Err(e) = job.await {
                tracing::warn!("dub of {id}: {e:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// After playing: the progress and what to continue moved on.
    pub fn reload_progress(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<i64> = self.seasons.keys().copied().collect();
        for id in ids {
            self.load_progress(id, cx);
        }
        if !self.franchise.is_empty() {
            let ids = self.franchise.iter().map(|p| p.id).collect();
            self.load_parts(ids, cx);
        }
    }

    fn load_progress(&mut self, id: i64, cx: &mut Context<Self>) {
        fetch(
            cx,
            self.core.run(move |c| async move {
                let anime = c.catalog.get(id).await?;
                let progress = c.library.progress(id)?;
                Ok(Progress {
                    entry: c.library.get_entry(id)?,
                    cont: feeds::continue_for(&anime, &progress),
                    progress,
                })
            }),
            move |page, progress| page.season_mut(id).progress = progress,
        );
    }

    /// Writes to the library, then reloads what it shows and tells the other screens.
    fn change(
        &mut self,
        cx: &mut Context<Self>,
        write: impl FnOnce(&Core) -> Result<(), ninari_core::error::AppError> + Send + 'static,
    ) {
        let id = self.shown;
        // What changed goes to the list on Shikimori too, when signed in.
        let job = self.core.run(move |c| async move {
            write(&c)?;
            c.account.push(id);
            Ok(())
        });
        cx.spawn(async move |page, cx| {
            if let Err(e) = job.await {
                tracing::warn!("library: {e:#}");
            }
            page.update(cx, |page, cx| {
                page.load_progress(id, cx);
                cx.emit(TitleEvent::Changed);
            })
            .ok();
        })
        .detach();
    }

    /// The colours of the title, once its poster is known.
    pub fn card(&self) -> Option<&AnimeCard> {
        match &self.cur().card {
            Load::Ready(c) => Some(c),
            _ => None,
        }
    }

    fn entry(&self) -> Option<&LibraryEntry> {
        match &self.cur().progress {
            Load::Ready(p) => p.entry.as_ref(),
            _ => None,
        }
    }

    /// The dub to play: picked here, remembered by the library, or the first found.
    fn dub_key(&self) -> Option<String> {
        let dubs = match &self.cur().sources {
            Load::Ready(s) => &s.dubs[..],
            _ => &[],
        };
        self.cur()
            .picked_dub
            .clone()
            .or_else(|| self.entry().and_then(|e| e.chosen_dub_key.clone()))
            .or_else(|| dubs.first().map(|d| d.key.clone()))
            .filter(|k| dubs.iter().any(|d| &d.key == k))
    }

    fn play(&self, episode: i64, cx: &mut Context<Self>) {
        // The old season is still drawn while a switch fades it out.
        if self.drawn != self.shown {
            return;
        }
        let (Some(card), Some(dub_key)) = (self.card(), self.dub_key()) else {
            return;
        };
        let request = Request {
            id: self.shown,
            episode,
            dub_key: Some(dub_key),
            start: None,
            title: title(&card.anime).into(),
            palette: poster(card, cx).map(|(_, p)| p).unwrap_or_default(),
        };
        cx.emit(TitleEvent::Play(request));
    }

    fn open(&self, route: Route, cx: &mut Context<Self>) -> OnClick {
        Box::new(cx.listener(move |_, _, _, cx| cx.emit(TitleEvent::Open(route))))
    }
}

/// A glass pill like the status and rating triggers.
fn pill(id: &'static str) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .cursor_pointer()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .pl(px(16.))
        .pr(px(20.))
        .py(px(13.))
        .rounded_full()
        .bg(theme::glass())
        .border_1()
        .border_color(theme::line())
        .relative()
        .hover(|s| s.bg(theme::glass_strong()).top(px(-1.)))
        .active(|s| s.top(px(1.)))
        .font_weight(FontWeight::BOLD)
}

impl Title {
    /// The facts in one line, then the genres.
    fn facts(&self, card: &AnimeCard, accent: Hsla, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let details = match &self.cur().details {
            Load::Ready(d) => Some(d),
            _ => None,
        };
        let mut line: Vec<AnyElement> = title_facts(&card.anime, details)
            .into_iter()
            .map(|fact| match fact {
                Fact::Score(text) => div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_color(rgb(theme::TEXT))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child(div().text_color(rgb(0xf5c451)).child("★"))
                    .child(text)
                    .into_any_element(),
                Fact::Status(text, tone) => {
                    let dot = div().size(px(7.)).rounded_full().map(|d| match tone {
                        Tone::Done => d.bg(rgb(0x7fe39a)),
                        Tone::Soon => d.bg(rgb(0xf0c867)),
                        Tone::Plain => d.bg(rgb(theme::TEXT_3)),
                        Tone::Live => d.bg(accent).shadow(vec![BoxShadow {
                            color: accent.opacity(0.25),
                            offset: point(px(0.), px(0.)),
                            blur_radius: px(0.),
                            spread_radius: px(3.),
                            inset: false,
                        }]),
                    });
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(dot)
                        .child(text)
                        .into_any_element()
                }
                Fact::Kind(text) | Fact::Episodes(text) => div().child(text).into_any_element(),
                Fact::Age(text) => div()
                    .px(px(6.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgb(theme::TEXT).opacity(0.35))
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child(text)
                    .into_any_element(),
            })
            .collect();
        if !card.anime.studios.is_empty() {
            line.push(
                links(
                    "studio",
                    &card.anime.studios,
                    ", ",
                    accent,
                    |i| studio_seed(&card.anime, i),
                    cx,
                )
                .into_any_element(),
            );
        }
        let mut out = vec![
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(px(16.))
                .gap_y(px(6.))
                .text_color(rgb(theme::TEXT_2))
                .font_weight(FontWeight::SEMIBOLD)
                .children(line)
                .into_any_element(),
        ];
        if !card.anime.genres.is_empty() {
            out.push(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .children(card.anime.genres.iter().enumerate().map(|(i, g)| {
                        let seed = genre_seed(&card.anime, i);
                        div()
                            .id(("genre", i))
                            .px(px(10.))
                            .py(px(3.))
                            .rounded_full()
                            .bg(theme::glass())
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::TEXT_2))
                            .when_some(seed, |d, seed| {
                                d.cursor_pointer()
                                    .hover(move |s| s.text_color(accent).bg(theme::glass_strong()))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(TitleEvent::Browse(seed.clone()))
                                    }))
                            })
                            .child(g.to_lowercase())
                    }))
                    .into_any_element(),
            );
        }
        out
    }

    fn actions(&self, accent: Hsla, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let progress = match &self.cur().progress {
            Load::Ready(p) => Some(p),
            _ => None,
        };
        let cont = progress.and_then(|p| p.cont.as_ref());
        let label = match cont {
            Some(c) if c.position > 0. => format!("Продолжить серию {}", c.episode),
            Some(c) => format!("Смотреть серию {}", c.episode),
            None => "Смотреть с 1 серии".into(),
        };
        let duration = progress
            .zip(cont)
            .and_then(|(p, c)| p.progress.iter().find(|e| e.episode == c.episode))
            .map_or(0., |e| e.duration_sec);
        let left = cont.and_then(|c| time_left(c.position, duration));
        let episode = cont.map_or(1, |c| c.episode);
        let can_play = self.card().is_some() && self.dub_key().is_some();
        let play = button(
            "play",
            label,
            Some("play"),
            Variant::Primary,
            Size::L,
            accent,
        )
        .children(left.map(|l| {
            div()
                .ml(px(2.))
                .opacity(0.7)
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("· {l}"))
        }))
        .map(|b| {
            if can_play {
                b.on_click(cx.listener(move |page, _, _, cx| page.play(episode, cx)))
            } else {
                b.opacity(0.55).cursor_default()
            }
        });

        let entry = self.entry();
        let status = entry.map(|e| e.status.clone());
        let in_list = status.as_deref().is_some_and(|s| s != "none");
        let menu = self.status_menu.clone();
        let status_trigger = div()
            .relative()
            .child(
                pill("status")
                    .on_click(cx.listener(move |_, e: &ClickEvent, _, cx| {
                        menu.toggle(e);
                        cx.notify();
                    }))
                    .child(
                        icon(if in_list { "check" } else { "plus" }, 18.)
                            .text_color(rgb(theme::TEXT)),
                    )
                    .child(match status.as_deref() {
                        Some(s) if in_list => library_status(s),
                        _ => "В список",
                    }),
            )
            .children(popover(
                &self.status_menu,
                ("status-menu", self.shown as u64),
                self.status_panel(status.as_deref(), accent, cx),
                window,
                cx,
            ));

        let score = entry.and_then(|e| e.score);
        let menu = self.rating_menu.clone();
        let rating_trigger = progress.map(|_| {
            div()
                .relative()
                .child(
                    pill("rating")
                        .on_click(cx.listener(move |page, e: &ClickEvent, _, cx| {
                            menu.toggle(e);
                            if menu.is_open() {
                                let saved = page.entry_note();
                                page.note.update(cx, |input, cx| input.set_text(saved, cx));
                            }
                            cx.notify();
                        }))
                        .child(
                            div()
                                .text_color(rgb(if score.is_some() {
                                    0xf5c451
                                } else {
                                    theme::TEXT_3
                                }))
                                .child(if score.is_some() { "★" } else { "☆" }),
                        )
                        .child(score.map_or("Оценить".to_string(), |s| s.to_string())),
                )
                .children(popover(
                    &self.rating_menu,
                    ("rating-menu", self.shown as u64),
                    self.rating_panel(score, entry.and_then(|e| e.note.clone()), accent, cx),
                    window,
                    cx,
                ))
        });

        let bounds = self.actions_bounds.clone();
        div()
            .relative()
            .flex()
            .flex_wrap()
            .gap(px(12.))
            .mt(px(12.))
            .child(play)
            .child(status_trigger)
            .children(rating_trigger)
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
    }

    fn status_panel(&self, current: Option<&str>, accent: Hsla, cx: &mut Context<Self>) -> Div {
        let item = |id: &'static str| {
            div()
                .id(id)
                .cursor_pointer()
                .flex()
                .justify_between()
                .items_center()
                .px(px(12.))
                .py(px(9.))
                .rounded(px(9.))
                .font_weight(FontWeight::SEMIBOLD)
                .hover(|s| s.bg(theme::glass_strong()))
        };
        let in_list = current.is_some_and(|s| s != "none");
        let id = self.shown;
        div()
            .min_w(px(220.))
            .flex()
            .flex_col()
            .children(STATUSES.map(|st| {
                let picked = current == Some(st);
                item(st)
                    .when(picked, |d| d.text_color(accent))
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.status_menu.close();
                        page.change(cx, move |c| c.library.set_status(id, st));
                    }))
                    .child(library_status(st))
                    .when(picked, |d| d.child(icon("check", 16.).text_color(accent)))
            }))
            .when(in_list, |d| {
                d.child(
                    item("remove")
                        .mt(px(4.))
                        .text_color(rgb(theme::TEXT_3))
                        .on_click(cx.listener(|page, _, _, cx| {
                            page.status_menu.close();
                            let id = page.shown;
                            page.change(cx, move |c| c.library.remove(id));
                        }))
                        .child("Убрать из списка"),
                )
            })
    }

    fn entry_note(&self) -> String {
        match &self.cur().progress {
            Load::Ready(p) => p.entry.as_ref().and_then(|e| e.note.clone()),
            _ => None,
        }
        .unwrap_or_default()
    }

    /// The note as typed goes to the library, when it differs from the saved one.
    fn save_note(&mut self, cx: &mut Context<Self>) {
        self.note_save = None;
        let typed = self.note.read(cx).text().trim().to_string();
        if typed == self.entry_note().trim() {
            return;
        }
        let Load::Ready(p) = &self.cur().progress else {
            return;
        };
        let (id, score) = (self.shown, p.entry.as_ref().and_then(|e| e.score));
        let note = (!typed.is_empty()).then_some(typed);
        self.change(cx, move |c| {
            c.library.set_rating(id, score, note.as_deref())
        });
    }

    fn open_fix(&mut self, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let id = self.shown;
        let fix = cx.new(|cx| SourceFix::new(core, id, cx));
        cx.subscribe(&fix, |page, _, event: &FixEvent, cx| {
            match event {
                FixEvent::Changed => {
                    let id = page.shown;
                    page.load_sources(id, cx)
                }
                FixEvent::Closed => page.fix = None,
            }
            cx.notify();
        })
        .detach();
        self.fix = Some(fix);
        cx.notify();
    }

    fn rating_panel(
        &self,
        score: Option<i64>,
        note: Option<String>,
        accent: Hsla,
        cx: &mut Context<Self>,
    ) -> Div {
        let shown = self.hovered_score.or(score);
        let scores = (1..=10).map(|n| {
            let lit = shown.is_some_and(|s| n <= s);
            let picked = score == Some(n);
            let note = note.clone();
            div()
                .id(("score", n as u64))
                .cursor_pointer()
                .size(px(34.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(10.))
                .font_weight(FontWeight::EXTRA_BOLD)
                .map(|d| {
                    if picked {
                        d.bg(accent).text_color(rgb(theme::ACCENT_CONTRAST))
                    } else if lit {
                        d.bg(accent.opacity(0.22)).text_color(rgb(theme::TEXT))
                    } else {
                        d.bg(theme::glass())
                            .border_1()
                            .border_color(theme::line())
                            .text_color(rgb(theme::TEXT_3))
                    }
                })
                .on_hover(cx.listener(move |page, hovered: &bool, _, cx| {
                    page.hovered_score = hovered
                        .then_some(n)
                        .or(page.hovered_score.filter(|h| *h != n));
                    cx.notify();
                }))
                .on_click(cx.listener(move |page, _, _, cx| {
                    let id = page.shown;
                    let next = (score != Some(n)).then_some(n);
                    let note = note.clone();
                    page.change(cx, move |c| c.library.set_rating(id, next, note.as_deref()));
                }))
                .child(n.to_string())
        });
        div()
            .w(px(440.))
            .p(px(10.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x(px(16.))
                    .gap_y(px(12.))
                    .child(div().flex().gap(px(4.)).children(scores))
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_2))
                            .text_size(px(13.))
                            .font_weight(FontWeight::BOLD)
                            .child(shown.map_or("Без оценки", |s| SCORE_WORDS[s as usize])),
                    ),
            )
            .child(self.note.clone())
    }
}

impl Title {
    /// "Серии" with the dub picker beside it, the tiles, and what went wrong with the sources.
    /// `enter` is the direction and the ms since a switch's new season started coming in.
    fn episodes(
        &self,
        width: gpui::Pixels,
        accent: Hsla,
        enter: Option<(f32, u64)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let sources = match &self.cur().sources {
            Load::Ready(s) => Some(s),
            _ => None,
        };
        let dubs = sources.map_or(&[][..], |s| &s.dubs[..]);
        let card = self.card();
        let aired = card.map_or(0, |c| {
            if c.anime.status.as_deref() == Some("released") {
                c.anime.episodes
            } else {
                c.anime.episodes_aired
            }
        });
        let dub_key = self.dub_key();
        let dub = dubs.iter().find(|d| Some(&d.key) == dub_key.as_ref());

        let picker = if dubs.is_empty() {
            matches!(self.cur().sources, Load::Pending).then(|| {
                div()
                    .w(px(220.))
                    .h(px(36.))
                    .rounded_full()
                    .bg(theme::glass())
                    .into_any_element()
            })
        } else {
            let total = dubs
                .iter()
                .filter_map(|d| d.episodes.last().copied())
                .fold(aired, i64::max);
            let options: Vec<SelectOption> = dubs
                .iter()
                .map(|d| {
                    let count = format!(
                        "{}/{}",
                        d.episodes.len(),
                        if total > 0 {
                            total
                        } else {
                            d.episodes.len() as i64
                        }
                    );
                    let hint = [
                        d.max_quality.map(|q| format!("{q}p")),
                        Some(count),
                        d.fuzzy.then(|| "найдено по названию".to_string()),
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · ");
                    SelectOption {
                        value: d.key.clone(),
                        label: d.name.clone(),
                        hint: Some(hint),
                    }
                })
                .collect();
            let page = cx.entity().downgrade();
            let value = dub_key.clone().unwrap_or_else(|| dubs[0].key.clone());
            Some(
                select(
                    "dub",
                    &self.dub_menu,
                    &value,
                    &options,
                    accent,
                    Rc::new(move |key, _, cx| {
                        page.update(cx, |page, cx| page.pick_dub(key, cx)).ok();
                    }),
                    window,
                    cx,
                )
                .into_any_element(),
            )
        };

        let mut body: Vec<AnyElement> = Vec::new();
        if let Load::Failed(e) = &self.cur().sources {
            body.push(
                div()
                    .text_color(rgb(theme::DANGER))
                    .text_size(px(13.))
                    .child(e.clone())
                    .into_any_element(),
            );
        }
        if sources.is_some_and(|s| s.dubs.is_empty() && s.errors.is_empty()) {
            body.push(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x(px(4.))
                    .text_color(rgb(theme::TEXT_2))
                    .child("Ни одно расширение не нашло этот тайтл. Можно")
                    .child(
                        div()
                            .id("plugins")
                            .cursor_pointer()
                            .text_color(accent)
                            .font_weight(FontWeight::BOLD)
                            .on_click(self.open(Route::Settings, cx))
                            .child("подключить расширения"),
                    )
                    .into_any_element(),
            );
        }
        match card {
            Some(c) => {
                let progress = match &self.cur().progress {
                    Load::Ready(p) => Some(p),
                    _ => None,
                };
                if let Some(d) = dub {
                    let next = next_up(
                        progress.and_then(|p| p.cont.as_ref()),
                        progress.map_or(&[][..], |p| &p.progress[..]),
                        &d.episodes,
                        c.anime.status.as_deref() == Some("released"),
                    );
                    let (title, left) = match &next {
                        NextUp::Play { episode, .. } => (
                            self.cur()
                                .marks
                                .iter()
                                .find(|m| m.number == *episode)
                                .and_then(|m| m.title.clone()),
                            progress
                                .and_then(|p| p.progress.iter().find(|e| e.episode == *episode))
                                .filter(|e| !e.watched)
                                .and_then(|e| time_left(e.position_sec, e.duration_sec)),
                        ),
                        _ => (None, None),
                    };
                    let waiting = next_episode_label(c.anime.next_episode_at.as_deref());
                    let page = cx.entity().downgrade();
                    let on_play: OnPlay = Rc::new(move |n, _, cx| {
                        page.update(cx, |page, cx| page.play(n, cx)).ok();
                    });
                    body.extend(next_card(
                        &next,
                        title.as_deref(),
                        left,
                        waiting,
                        accent,
                        on_play,
                    ));
                }
                let tiles = episode_tiles(
                    c.anime.episodes,
                    aired,
                    dub.map_or(&[][..], |d| &d.episodes[..]),
                    progress.map_or(&[][..], |p| &p.progress[..]),
                    progress.and_then(|p| p.cont.as_ref()).map(|c| c.episode),
                );
                let page = cx.entity().downgrade();
                let on_play = Rc::new(move |n, _: &mut Window, cx: &mut gpui::App| {
                    page.update(cx, |page, cx| page.play(n, cx)).ok();
                });
                let page = cx.entity().downgrade();
                let on_block = Rc::new(move |i, _: &mut Window, cx: &mut gpui::App| {
                    page.update(cx, |page, cx| {
                        let id = page.shown;
                        page.season_mut(id).episode_block = Some(i as usize);
                        cx.notify();
                    })
                    .ok();
                });
                body.push(
                    episode_grid(
                        &tiles,
                        &self.cur().marks,
                        self.cur().episode_block,
                        width,
                        accent,
                        enter,
                        on_play,
                        on_block,
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            None => body.push(
                div()
                    .h(px(100.))
                    .rounded(px(12.))
                    .bg(theme::glass())
                    .into_any_element(),
            ),
        }
        if let Some(s) = sources
            && !s.errors.is_empty()
        {
            body.push(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .children(s.errors.iter().enumerate().map(|(i, e)| {
                        chip(("source-error", i), false, accent)
                            .text_color(rgb(0xf0c867))
                            .on_click(cx.listener(|page, _, _, cx| {
                                let id = page.shown;
                                page.season_mut(id).sources = Load::Pending;
                                page.load_sources(id, cx);
                                cx.notify();
                            }))
                            .child(icon("retry", 14.).text_color(rgb(0xf0c867)))
                            .child(format!("{} недоступен, повторить", e.extension_id))
                    }))
                    .into_any_element(),
            );
        }
        if let Some(d) = dub {
            let list: Vec<String> = d
                .sources
                .iter()
                .map(|s| {
                    let q = s.max_quality.map(|q| format!(" {q}p")).unwrap_or_default();
                    format!("{}{q}", s.extension_id)
                })
                .collect();
            body.push(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .text_color(rgb(theme::TEXT_3))
                    .text_size(px(13.))
                    .child(format!("Источники: {}.", list.join(", ")))
                    .child(
                        div()
                            .id("fix-sources")
                            .cursor_pointer()
                            .font_weight(FontWeight::BOLD)
                            .text_color(accent)
                            .hover(|s| s.underline())
                            .on_click(cx.listener(|page, _, _, cx| page.open_fix(cx)))
                            .child("Не тот тайтл?"),
                    )
                    .into_any_element(),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x(px(16.))
                    .gap_y(px(12.))
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child("Серии"),
                    )
                    .children(picker),
            )
            .children(body)
    }
}

/// Names joined by `sep`, each a link to `route(i)`.
fn genre_seed(a: &Anime, i: usize) -> Option<Seed> {
    a.genres.get(i).cloned().map(Seed::Genre)
}

/// Studios without a known id are left as plain names.
fn studio_seed(a: &Anime, i: usize) -> Option<Seed> {
    Some(Seed::Studio(
        *a.studio_ids.get(i)?,
        a.studios.get(i)?.clone(),
    ))
}

fn links(
    key: &'static str,
    names: &[String],
    sep: &'static str,
    accent: Hsla,
    seed: impl Fn(usize) -> Option<Seed>,
    cx: &mut Context<Title>,
) -> Div {
    div()
        .flex()
        .flex_wrap()
        .children(names.iter().enumerate().flat_map(|(i, name)| {
            let link = match seed(i) {
                Some(seed) => div()
                    .id((key, i))
                    .cursor_pointer()
                    .hover(move |s| s.text_color(accent))
                    .on_click(
                        cx.listener(move |_, _, _, cx| cx.emit(TitleEvent::Browse(seed.clone()))),
                    )
                    .child(name.clone())
                    .into_any_element(),
                None => div().child(name.clone()).into_any_element(),
            };
            let sep = (i + 1 < names.len())
                .then(|| div().whitespace_nowrap().child(sep).into_any_element());
            [Some(link), sep].into_iter().flatten()
        }))
}

impl Render for Title {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (accent, glow) = Accent::colors(cx);
        let now = Instant::now();
        let card_in = matches!(self.seasons[&self.shown].card, Load::Ready(_));
        if let Some(s) = &mut self.switch
            && card_in
            && s.ready.is_none()
        {
            s.ready = Some(now - s.started);
        }
        let phase = match &self.switch {
            Some(s) => phase(
                (now - s.started).as_millis() as u64,
                s.ready.map(|r| r.as_millis() as u64),
            ),
            None => Phase::Done,
        };
        if phase == Phase::Done {
            self.switch = None;
        } else {
            window.request_animation_frame();
        }
        let dir = self.switch.as_ref().map_or(1., |s| s.dir);
        self.drawn = match (&self.switch, phase) {
            (Some(s), Phase::Out(_) | Phase::Hold) => s.from,
            _ => self.shown,
        };
        let page = div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(48.))
            .px(px(48.))
            .pt(px(50.))
            .pb(px(72.));
        if let Load::Failed(e) = &self.cur().card {
            return page.child(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(12.))
                    .pt(px(48.))
                    .child(
                        div()
                            .text_size(px(34.))
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child("Не получилось открыть тайтл"),
                    )
                    .child(div().text_color(rgb(theme::TEXT_2)).child(e.clone()))
                    .child(
                        button(
                            "retry",
                            "Повторить",
                            Some("retry"),
                            Variant::Glass,
                            Size::M,
                            accent,
                        )
                        .on_click(cx.listener(|page, _, _, cx| {
                            let id = page.shown;
                            *page.season_mut(id) = Season::new();
                            page.load(id, cx);
                            cx.notify();
                        })),
                    ),
            );
        }

        for old in self.stale.drain(..) {
            window.drop_image(old).ok();
        }
        // Only the part on screen, and the one a switch fades out, keep their banner.
        let fading = self.switch.as_ref().map(|s| s.from);
        let shown = self.shown;
        self.drop_banners(window, |id| id == shown || Some(id) == fading);
        let width = window.viewport_size().width - px(rail::WIDTH);
        let want = f32::from(width).round() as u32;
        let shown = self.shown;
        let season = self.season_mut(shown);
        if let Some(source) = season.banner.clone()
            && season.banner_view.as_ref().is_none_or(|(w, _)| *w != want)
            && season.banner_pending != Some(want)
        {
            season.banner_pending = Some(want);
            let job = self.core.rt.spawn_blocking(move || {
                let decoded = image::open(&source).map_err(|e| tracing::info!("banner: {e}"));
                decoded
                    .ok()
                    .map(|i| fade_banner(&i.into_rgba8(), want as f32, 440.))
            });
            cx.spawn(async move |page, cx| {
                let Ok(Some(faded)) = job.await else { return };
                page.update(cx, |page, cx| {
                    let season = page.season_mut(shown);
                    if season.banner_pending == Some(want) {
                        season.banner_pending = None;
                    }
                    if let Some((_, old)) = season.banner_view.replace((want, faded)) {
                        page.stale.push(old);
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        // The old season's banner fades out under the new one.
        let under = self.switch.as_ref().and_then(|s| {
            let fade = 1. - (s.started.elapsed().as_secs_f32() / 0.4).min(1.);
            let (_, view) = self.seasons.get(&s.from)?.banner_view.clone()?;
            (fade > 0.).then(|| {
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w(width)
                    .h(px(440.))
                    .opacity(fade)
                    .child(img(view).size_full().object_fit(ObjectFit::Cover))
            })
        });
        let banner = self.seasons[&self.shown]
            .banner_view
            .clone()
            .map(|(_, faded)| {
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w(width)
                    .h(px(440.))
                    .child(
                        img(faded)
                            .size_full()
                            .object_fit(ObjectFit::Cover)
                            .with_animation(
                                ("banner-in", (self.shown as u64) << 8 | (self.nonce & 0xff)),
                                Animation::new(theme::T_SLOW).with_easing(theme::ease),
                                |img, t| img.opacity(t),
                            ),
                    )
            });

        let card = self.card().cloned();
        let cover = match &card {
            Some(c) => poster_frame(c, px(220.), theme::R_M, cx)
                .shadow(vec![BoxShadow {
                    color: glow.opacity(0.8),
                    offset: point(px(0.), px(30.)),
                    blur_radius: px(80.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .into_any_element(),
            None => div()
                .flex_none()
                .w(px(220.))
                .h(px(330.))
                .rounded(px(18.))
                .bg(theme::glass())
                .into_any_element(),
        };

        let mut info: Vec<AnyElement> = Vec::new();
        match &card {
            Some(c) => {
                info.push(
                    div()
                        .text_size(px(36.))
                        .line_height(px(42.))
                        .line_clamp(2)
                        .text_ellipsis()
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .child(title(&c.anime))
                        .into_any_element(),
                );
                let japanese = match &self.cur().details {
                    Load::Ready(d) => d.japanese.clone(),
                    _ => None,
                };
                if c.anime.russian.as_ref().is_some_and(|r| *r != c.anime.name) {
                    info.push(
                        div()
                            .mt(px(-6.))
                            .flex()
                            .flex_wrap()
                            .text_color(rgb(theme::TEXT_2))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(c.anime.name.clone())
                            .children(japanese.map(|j| {
                                div()
                                    .text_color(rgb(theme::TEXT_3))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(format!("  ·  {j}"))
                            }))
                            .into_any_element(),
                    );
                }
                info.extend(self.facts(c, accent, cx));
                if c.anime.status.as_deref() == Some("ongoing")
                    && let Some(next) = next_episode_label(c.anime.next_episode_at.as_deref())
                {
                    info.push(
                        div()
                            .text_color(theme::mix(rgb(theme::TEXT), accent, 0.7))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(13.))
                            .child(next)
                            .into_any_element(),
                    );
                }
            }
            None => {
                for (w, h) in [(0.6, 44.), (0.3, 20.)] {
                    info.push(
                        div()
                            .w(relative(w))
                            .h(px(h))
                            .rounded(px(8.))
                            .bg(theme::glass())
                            .into_any_element(),
                    );
                }
            }
        }
        info.push(self.actions(accent, window, cx).into_any_element());

        if let (Load::Ready(p), Some(c)) = (&self.cur().progress, &card) {
            let watched = p.progress.iter().filter(|e| e.watched).count() as i64;
            let aired = if c.anime.status.as_deref() == Some("released") {
                c.anime.episodes
            } else {
                c.anime.episodes_aired
            };
            let total = if c.anime.episodes > 0 {
                c.anime.episodes
            } else {
                aired
            };
            if watched > 0 && total > 0 {
                let shown = watched.min(total);
                info.push(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .text_color(rgb(theme::TEXT_3))
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(
                            div()
                                .w(px(160.))
                                .h(px(4.))
                                .rounded_full()
                                .overflow_hidden()
                                .bg(theme::glass_strong())
                                .child(
                                    div()
                                        .h_full()
                                        .w(px(160. * shown as f32 / total as f32))
                                        .rounded_full()
                                        .bg(accent),
                                ),
                        )
                        .child(format!("Просмотрено {shown} из {total}"))
                        .into_any_element(),
                );
            }
        }

        let info = div()
            .flex_1()
            .min_w(px(0.))
            .max_w(px(820.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .children(info.into_iter().enumerate().map(|(i, el)| {
                rise(
                    el,
                    ("title-info", (self.opened as u64) << 4 | i as u64),
                    i,
                    620,
                    50,
                    14.,
                )
            }));

        let content = (window.viewport_size().width - px(rail::WIDTH)).min(px(1400.)) - px(96.);
        // The franchise is the same from every part: kept once, so the seasons row stays put
        // while a switched-to part's details are still on the way.
        let found = self
            .seasons
            .values()
            .find_map(|s| match &s.details {
                Load::Ready(d) if d.parts.len() > 1 => Some(d.parts.clone()),
                _ => None,
            })
            .filter(|_| self.franchise.is_empty());
        if let Some(parts) = found {
            let ids = parts.iter().map(|p| p.id).collect();
            self.franchise = parts;
            self.load_parts(ids, cx);
        }
        let details = match &self.cur().details {
            Load::Ready(d) => Some(d),
            _ => None,
        };
        let seasons = self.seasons(accent, window, cx);
        let enter = match phase {
            Phase::In { ms, .. } => Some((dir, ms)),
            _ => None,
        };
        // The episodes, the description, the videos and similar titles are built only near the
        // screen: the page laid all of it out on every frame.
        let screen = lazy_grid::screen(window);
        let near = self.blocks.clone().map(|b| b.near(screen));
        let episodes = near[0].then(|| self.episodes(content, accent, enter, window, cx));
        let about = card.as_ref().filter(|_| near[1]).map(|c| {
            let link = |key,
                        names: &[String],
                        seed: &dyn Fn(usize) -> Option<Seed>,
                        cx: &mut Context<Self>| {
                (!names.is_empty())
                    .then(|| links(key, names, ", ", accent, seed, cx).into_any_element())
            };
            let genres = link(
                "about-genre",
                &c.anime.genres,
                &|i| genre_seed(&c.anime, i),
                cx,
            );
            let studios = link(
                "about-studio",
                &c.anime.studios,
                &|i| studio_seed(&c.anime, i),
                cx,
            );
            let page = cx.entity().downgrade();
            let on_hero: crate::episode_grid::OnNumber = std::rc::Rc::new(move |id, _, cx| {
                page.update(cx, |_, cx| cx.emit(TitleEvent::Open(Route::Character(id))))
                    .ok();
            });
            let key = ("about-more", self.opened as u64);
            motion::set(
                key,
                self.about_open,
                Duration::from_millis(400),
                theme::EASE,
                cx,
            );
            let page = cx.entity().downgrade();
            let more = More {
                open: motion::get(key, window, cx),
                opened: self.about_open,
                height: self.about_h.clone(),
                toggle: Rc::new(move |_, cx| {
                    page.update(cx, |page, cx| {
                        page.about_open = !page.about_open;
                        cx.notify();
                    })
                    .ok();
                }),
            };
            about(c, details, content, genres, studios, accent, more, on_hero)
        });
        let videos = details
            .filter(|_| near[2])
            .and_then(|d| videos(&d.videos, &self.video_rows, accent, window, cx));
        let similar = near[3]
            .then(|| self.similar_section(details, accent, window, cx))
            .flatten();
        let sticky = card
            .as_ref()
            .and_then(|c| self.sticky_bar(c, accent, window, cx));
        let id = self.opened as u64;
        // What is below the episodes only fades: it is far from the eye during a switch.
        let rest = match phase {
            Phase::In { t, .. } => theme::EASE.at(t),
            Phase::Done => 1.,
            _ => 0.,
        };
        page.children(under)
            .children(banner)
            .children(self.fix.clone())
            .child(slide(
                div()
                    .flex()
                    .items_start()
                    .gap(px(32.))
                    .pt(px(24.))
                    .child(cover)
                    .child(info),
                phase,
                dir,
            ))
            .children(seasons.map(|s| rise(s, ("title-seasons", id), 1, 620, 200, 14.)))
            .child(self.blocks[0].slot(
                near[0],
                episodes.map(|e| {
                    rise(
                        slide(e, phase, dir),
                        ("title-episodes", id),
                        1,
                        620,
                        240,
                        14.,
                    )
                }),
            ))
            .child(self.blocks[1].slot(
                near[1],
                about.map(|a| {
                    rise(
                        div().opacity(rest).child(a),
                        ("title-about", id),
                        1,
                        620,
                        340,
                        14.,
                    )
                }),
            ))
            .child(self.blocks[2].slot(
                near[2],
                videos.map(|v| div().opacity(rest).child(v).into_any_element()),
            ))
            .child(self.blocks[3].slot(
                near[3],
                similar.map(|v| div().opacity(rest).child(v).into_any_element()),
            ))
            .children(sticky)
    }
}

impl Title {
    /// Seasons, films and OVAs of the franchise as cards, each shown in place.
    fn seasons(&self, accent: Hsla, window: &mut Window, cx: &mut Context<Self>) -> Option<Div> {
        let chips = part_chips(&self.franchise, self.shown);
        if chips.is_empty() {
            return None;
        }
        let loaded = match &self.parts {
            Load::Ready(list) => &list[..],
            _ => &[],
        };
        let cards: Vec<PartCard> = chips
            .into_iter()
            .map(|chip| {
                let found = loaded.iter().find(|p| p.0 == chip.id);
                PartCard {
                    card: found.and_then(|p| p.1.clone()),
                    watched: found.and_then(|p| p.2),
                    chip,
                }
            })
            .collect();
        let page = cx.entity().downgrade();
        let on_part: OnPart = Rc::new(move |id, _, cx| {
            page.update(cx, |page, cx| {
                page.show(id, cx);
                cx.emit(TitleEvent::Season(id));
            })
            .ok();
        });
        Some(season_cards(&cards, accent, on_part, window, cx))
    }

    /// The franchise's parts once: their cards for the posters, and watched / episodes of
    /// those in the library.
    fn load_parts(&mut self, ids: Vec<i64>, cx: &mut Context<Self>) {
        fetch(
            cx,
            self.core.run(move |c| async move {
                let animes = c.catalog.get_many(&ids).await?;
                let mut out = Vec::new();
                for id in ids {
                    let card = animes
                        .iter()
                        .find(|a| a.id == id)
                        .cloned()
                        .map(|a| feeds::card(&c.posters, a));
                    let watched = match c.library.get_entry(id)? {
                        Some(_) => {
                            let seen = c.library.progress(id)?.iter().filter(|p| p.watched).count();
                            let total = card
                                .as_ref()
                                .map_or(0, |c| c.anime.episodes.max(c.anime.episodes_aired));
                            Some((seen as i64, total))
                        }
                        None => None,
                    };
                    out.push((id, card, watched));
                }
                Ok(out)
            }),
            |page, parts| {
                // A reload keeps what is shown until the new list is in.
                if matches!(parts, Load::Ready(_)) || matches!(page.parts, Load::Pending) {
                    page.parts = parts;
                }
            },
        );
    }

    /// Titles Shikimori finds similar, without the parts of this title's own franchise.
    fn similar_section(
        &self,
        details: Option<&TitleDetails>,
        accent: Hsla,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let Load::Ready(list) = &self.cur().similar else {
            return None;
        };
        let own: Vec<i64> = details.map_or(Vec::new(), |d| d.parts.iter().map(|p| p.id).collect());
        let cards: Vec<AnyElement> = list
            .iter()
            .filter(|c| c.anime.id != self.shown && !own.contains(&c.anime.id))
            .enumerate()
            .map(|(i, c)| {
                let text = CardText {
                    note: None,
                    fraction: None,
                    badge: None,
                };
                let open = self.open(Route::Title(c.anime.id), cx);
                poster_card(c, "similar", i, text, accent, open, window, cx).into_any_element()
            })
            .collect();
        if cards.is_empty() {
            return None;
        }
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(16.))
                .child(
                    div()
                        .text_size(px(18.))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .child("Похожее"),
                )
                .child(self.similar_row.render(
                    ("similar", self.shown as u64),
                    px(24.),
                    (14., 44., 34.),
                    cards,
                    window,
                    cx,
                )),
        )
    }

    /// A slim bar with the title and its main button, fading in over the page once the
    /// header's buttons have scrolled under it.
    fn sticky_bar(
        &self,
        card: &AnimeCard,
        accent: Hsla,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        const HEIGHT: f32 = 56.;
        let bounds = self.actions_bounds.get();
        let gone = bounds.size.height > px(0.) && bounds.bottom() <= px(HEIGHT);
        let key = ("sticky", self.shown as u64);
        motion::set(
            key,
            gone,
            std::time::Duration::from_millis(180),
            theme::EASE,
            cx,
        );
        let shown = motion::get(key, window, cx);
        if shown <= 0. {
            return None;
        }
        let cont = match &self.cur().progress {
            Load::Ready(p) => p.cont.as_ref(),
            _ => None,
        };
        let label = match cont {
            Some(c) if c.position > 0. => format!("Продолжить серию {}", c.episode),
            Some(c) => format!("Смотреть серию {}", c.episode),
            None => "Смотреть с 1 серии".into(),
        };
        let episode = cont.map_or(1, |c| c.episode);
        let can_play = self.dub_key().is_some();
        // A floating island like the catalog's pinned controls, sliding down into place.
        let width = window.viewport_size().width - px(rail::WIDTH + 48.);
        let bar = div()
            .occlude()
            .w(width)
            .h(px(HEIGHT))
            .pl(px(12.))
            .pr(px(10.))
            .flex()
            .items_center()
            .gap(px(12.))
            .rounded(theme::R_L)
            // GPUI cannot blur what lies under an element: dense enough not to need it.
            .bg(rgba((theme::INK << 8) | 0xf6))
            .border_1()
            .border_color(theme::line())
            .shadow(vec![BoxShadow {
                color: rgba(0x00000080).into(),
                offset: point(px(0.), px(12.)),
                blur_radius: px(32.),
                spread_radius: px(-8.),
                inset: false,
            }])
            .opacity(shown.min(1.))
            .child(poster_frame(card, px(30.), px(6.), cx))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child(title(&card.anime)),
            )
            .child(
                button(
                    "sticky-play",
                    label,
                    Some("play"),
                    Variant::Primary,
                    Size::M,
                    accent,
                )
                .map(|b| {
                    if can_play {
                        b.on_click(cx.listener(move |page, _, _, cx| page.play(episode, cx)))
                    } else {
                        b.opacity(0.55).cursor_default()
                    }
                }),
            );
        Some(
            deferred(
                anchored()
                    .position_mode(AnchoredPositionMode::Window)
                    .position(point(px(rail::WIDTH + 24.), px(10. - 8. * (1. - shown))))
                    .child(bar),
            )
            .into_any_element(),
        )
    }
}

/// The header or the episodes during a switch: out against the direction, in from it.
fn slide(el: impl IntoElement, phase: Phase, dir: f32) -> Div {
    use crate::season_switch::{IN_SHIFT, OUT_SHIFT};
    let (opacity, x) = match phase {
        Phase::Out(t) => (1. - t, -dir * OUT_SHIFT * t),
        Phase::Hold => (0., -dir * OUT_SHIFT),
        Phase::In { t, .. } => {
            let e = theme::EASE.at(t);
            (e, dir * IN_SHIFT * (1. - e))
        }
        Phase::Done => (1., 0.),
    };
    div().relative().left(px(x)).opacity(opacity).child(el)
}

/// The banner cut to a `width` x `height` box, covering it with the picture's centre 30% from the
/// top, at half brightness, and fading out towards the bottom and the rail, so the page's glow
/// shows through.
fn fade_banner(source: &RgbaImage, width: f32, height: f32) -> Arc<RenderImage> {
    let (sw, sh) = (source.width() as f32, source.height() as f32);
    let aspect = width / height;
    let (cw, ch) = if sw / sh > aspect {
        (sh * aspect, sh)
    } else {
        (sw, sw / aspect)
    };
    let (x0, y0) = ((sw - cw) / 2., (sh - ch) * 0.3);
    let mut out =
        image::imageops::crop_imm(source, x0 as u32, y0 as u32, cw as u32, ch as u32).to_image();
    let (w, h) = (out.width() as f32, out.height() as f32);
    for (x, y, p) in out.enumerate_pixels_mut() {
        let down = 1. - ((y as f32 / h - 0.35) / 0.65).clamp(0., 1.);
        let side = (x as f32 / w / 0.12).min(1.);
        // Half bright under the text on the left, brighter towards the art on the right.
        let light = 0.5 + 0.25 * (x as f32 / w);
        let [r, g, b, a] = p.0;
        // BGRA, as GPUI keeps images.
        p.0 = [
            (b as f32 * light) as u8,
            (g as f32 * light) as u8,
            (r as f32 * light) as u8,
            (a as f32 * down * side) as u8,
        ];
    }
    Arc::new(RenderImage::new(vec![Frame::new(out)]))
}
