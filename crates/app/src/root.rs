//! The window's root: the rail and the page on screen, or the player over everything while an
//! episode plays.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, AnyView, Context, Entity, EventEmitter, FocusHandle, FontWeight, IntoElement,
    KeyDownEvent, MouseButton, NavigationDirection, Pixels, Transformation, Window, div,
    image_cache, prelude::*, px, relative, rgb, rgba, size, svg,
};
use ninari_core::catalog::posters::Palette;
use ninari_core::error::AppError;

use crate::button::{Size, Variant, button};
use crate::catalog_page::{CatalogPage, Seed};
use crate::core::Core;
use crate::episode_grid::chip;
use crate::history_page::HistoryPage;
use crate::home::{Home, HomeEvent};
use crate::hover_card;
use crate::images::Images;
use crate::library_page::{LibraryPage, Open};
use crate::modal::modal;
#[cfg(target_os = "linux")]
use crate::mpv::Mpv;
use crate::nav::PageEvent;
use crate::nav::{Back, Forward, History, Route};
use crate::onboarding::{Done, ONBOARDED_SETTING, Onboarding};
use crate::palette::{CommandPalette, PaletteEvent};
use crate::people_page::{CharacterPage, PersonPage};
use crate::playback::HostMsg;
use crate::player::{Closed, Player, Request};
use crate::popover;
use crate::posters::poster;
use crate::rail;
use crate::random::{self, RandomFilter, RandomState};
use crate::repo_offer::{Closed as OfferClosed, RepoOffer};
use crate::schedule_page::SchedulePage;
use crate::settings_page::SettingsPage;
use crate::smooth::{Autoscroll, Glide};
use crate::stats_page::StatsPage;
use crate::theme::{self, Accent};
use crate::title::{Title, TitleEvent};
use crate::updates::{self, Progress, UpdateInfo};
#[cfg(target_os = "linux")]
use crate::video::Video;
#[cfg(windows)]
use crate::video_win::VideoWindow;

pub struct Root {
    core: Arc<Core>,
    home: Entity<Home>,
    /// The first start's steps, over everything until done.
    onboarding: Option<Entity<Onboarding>>,
    /// Ctrl+K's quick jump, while open.
    palette: Option<Entity<CommandPalette>>,
    /// A plugin repository an `ninari://` link offers.
    offer: Option<Entity<RepoOffer>>,
    /// A newer version found after start, offered in the corner.
    update: Option<UpdateInfo>,
    history: History,
    /// Title pages opened so far, kept so going back finds them as they were.
    titles: HashMap<i64, Entity<Title>>,
    /// Character and person pages opened so far.
    people: HashMap<Route, AnyView>,
    library: Option<Entity<LibraryPage>>,
    catalog: Option<Entity<CatalogPage>>,
    schedule: Option<Entity<SchedulePage>>,
    history_page: Option<Entity<HistoryPage>>,
    stats: Option<Entity<StatsPage>>,
    settings: Option<Entity<SettingsPage>>,
    /// The search field takes the keyboard on the next frame (the page was just opened).
    focus_search: bool,
    player: Option<Entity<Player>>,
    /// libmpv and the surface it draws into, once the player asked for them.
    #[cfg(target_os = "linux")]
    video: Option<(Video, Arc<Mpv>)>,
    /// Windows: the child window under the app that mpv draws into, shown while the player is.
    #[cfg(windows)]
    video_window: Option<VideoWindow>,
    focus: FocusHandle,
    glide: Glide,
    autoscroll: Autoscroll,
    /// Every picture of the window, kept decoded up to a budget.
    images: Entity<Images>,
    /// The title page drawn last: it frees its banners once another page is on screen.
    drawn_title: Option<Entity<Title>>,
    random: RandomState,
    /// The page's scroll on the last frame.
    last_offset: Option<Pixels>,
}

impl Root {
    /// `autoplay` starts the title on the continue card as soon as the home screen is in.
    pub fn new(
        core: Arc<Core>,
        autoplay: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let home = cx.new(|cx| Home::new(core.clone(), autoplay, cx));
        cx.subscribe_in(
            &home,
            window,
            |root, _, event: &HomeEvent, window, cx| match event {
                HomeEvent::Open(route) => root.open(*route, cx),
                HomeEvent::Play(request) => root.play(request.clone(), window, cx),
            },
        )
        .detach();
        let onboarding = (!autoplay && core.setting::<bool>(ONBOARDED_SETTING).is_none())
            .then(|| onboarding(&core, window, cx));
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        // Anime4K is picked for the screen the window is on.
        if let Some(display) = window.display(cx) {
            let height = f32::from(display.bounds().size.height) * window.scale_factor();
            core.player_host.set_screen_height(height as u32);
        }
        #[cfg(windows)]
        crate::win_move::hold_moves(&*window);
        #[cfg(windows)]
        let video_window = VideoWindow::new(&*window)
            .inspect(|v| core.player_host.set_video_window(v.handle()))
            .map_err(|e| tracing::warn!("no video window, mpv plays in its own: {e:#}"))
            .ok();
        let random_filter = core.setting(random::SETTING).unwrap_or_default();
        let images = Images::new(cx);
        cx.set_global(crate::images::Current(images.clone()));
        Self {
            core,
            home,
            onboarding,
            palette: None,
            offer: None,
            update: None,
            history: History::new(Route::Home),
            titles: HashMap::new(),
            people: HashMap::new(),
            library: None,
            catalog: None,
            schedule: None,
            history_page: None,
            stats: None,
            settings: None,
            focus_search: false,
            player: None,
            #[cfg(target_os = "linux")]
            video: None,
            #[cfg(windows)]
            video_window,
            focus,
            glide: Glide::default(),
            autoscroll: Autoscroll::default(),
            images,
            drawn_title: None,
            last_offset: None,
            random: RandomState {
                filter: random_filter,
                ..Default::default()
            },
        }
    }

    /// A page's wishes: go somewhere, play, or tell the other lists the library changed.
    fn on_page_events<V: EventEmitter<PageEvent>>(
        &mut self,
        page: &Entity<V>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.subscribe_in(
            page,
            window,
            |root, _, event: &PageEvent, window, cx| match event {
                PageEvent::Open(route) => root.open(*route, cx),
                PageEvent::Play(request) => root.play(request.clone(), window, cx),
                PageEvent::Changed => root.library_changed(cx),
                PageEvent::Random => root.roll_random(cx),
                PageEvent::Top => {
                    root.glide.stop();
                    root.autoscroll.stop();
                    let scroll = root.history.current().scroll.clone();
                    scroll.set_offset(gpui::point(scroll.offset().x, gpui::px(0.)));
                    cx.notify();
                }
            },
        )
        .detach();
    }

    /// Opens a random title by the random filter; from the random title, another one.
    pub fn roll_random(&mut self, cx: &mut Context<Self>) {
        if self.random.rolling {
            return;
        }
        self.random.rolling = true;
        self.random.note = None;
        let job = random::roll(&self.core, &self.random.filter, self.random.seen.clone());
        cx.spawn(async move |root, cx| {
            let got = job.await;
            root.update(cx, |root, cx| {
                root.random.rolling = false;
                match got {
                    Ok(Some(id)) => {
                        root.random.seen.insert(id);
                        root.random.current = Some(id);
                        root.open(Route::Title(id), cx);
                    }
                    Ok(None) => root.random.note = Some("Под эти условия ничего не нашлось".into()),
                    Err(e) => root.random.note = Some(format!("Не вышло: {e:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn set_random(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut RandomFilter)) {
        change(&mut self.random.filter);
        // The titles shown so far were picked by other conditions.
        self.random.seen.clear();
        if let Ok(value) = serde_json::to_value(&self.random.filter) {
            self.core.save_setting(random::SETTING, value);
        }
        cx.notify();
    }

    fn open_random_settings(&mut self, cx: &mut Context<Self>) {
        self.random.settings = true;
        if self.random.genres.is_none() {
            let job = self.core.run(|c| async move { c.catalog.genres().await });
            cx.spawn(async move |root, cx| {
                if let Ok(list) = job.await {
                    root.update(cx, |root, cx| {
                        root.random.genres =
                            Some(list.into_iter().filter(|g| g.kind == "genre").collect());
                        cx.notify();
                    })
                    .ok();
                }
            })
            .detach();
        }
        cx.notify();
    }

    /// Along the top of the random title (or of any page, after a roll that found nothing):
    /// another one, and what it may be.
    fn random_bar(&self, route: Route, cx: &mut Context<Self>) -> Option<AnyElement> {
        let on_it = matches!(route, Route::Title(id) if self.random.current == Some(id));
        if !on_it && self.random.note.is_none() {
            return None;
        }
        let accent = Accent::colors(cx).0;
        let count = self.random.filter.count();
        Some(
            div()
                .occlude()
                .absolute()
                .top(px(10.))
                .right(px(24.))
                .flex()
                .items_center()
                .gap(px(8.))
                .pl(px(16.))
                .pr(px(6.))
                .py(px(6.))
                .rounded(theme::R_L)
                .bg(rgba((theme::INK << 8) | 0xec))
                .border_1()
                .border_color(theme::line())
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(theme::TEXT_2))
                        .child(
                            self.random
                                .note
                                .clone()
                                .unwrap_or_else(|| "Случайный тайтл".into()),
                        ),
                )
                .child(
                    button(
                        "random-again",
                        if self.random.rolling {
                            "Ищем…"
                        } else {
                            "Ещё раз"
                        },
                        Some("shuffle"),
                        Variant::Primary,
                        Size::M,
                        accent,
                    )
                    .on_click(cx.listener(|root, _, _, cx| root.roll_random(cx))),
                )
                .child(
                    button(
                        "random-settings",
                        if count > 0 {
                            format!("Условия · {count}")
                        } else {
                            "Условия".into()
                        },
                        Some("filter"),
                        Variant::Glass,
                        Size::M,
                        accent,
                    )
                    .on_click(cx.listener(|root, _, _, cx| root.open_random_settings(cx))),
                )
                .into_any_element(),
        )
    }

    /// What a random title may be: genres, years and a studio, kept between runs.
    fn random_settings(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let accent = Accent::colors(cx).0;
        let f = self.random.filter.clone();
        let row = |label: &'static str, chips: Vec<AnyElement>| {
            div()
                .flex()
                .gap(px(16.))
                .child(
                    div()
                        .w(px(70.))
                        .flex_none()
                        .pt(px(6.))
                        .text_size(px(13.))
                        .text_color(rgb(theme::TEXT_3))
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .children(chips),
                )
        };
        let genres: Vec<AnyElement> = match &self.random.genres {
            None => vec![
                div()
                    .text_size(px(13.))
                    .text_color(rgb(theme::TEXT_3))
                    .pt(px(6.))
                    .child("Загружаем жанры")
                    .into_any_element(),
            ],
            Some(list) => list
                .iter()
                .map(|g| {
                    let id = g.id;
                    chip(("random-genre", id as u64), f.genres.contains(&id), accent)
                        .on_click(cx.listener(move |root, _, _, cx| {
                            root.set_random(cx, |f| {
                                if f.genres.contains(&id) {
                                    f.genres.retain(|&g| g != id);
                                } else {
                                    f.genres.push(id);
                                }
                            })
                        }))
                        .child(g.name.clone())
                        .into_any_element()
                })
                .collect(),
        };
        let mut years = vec![
            chip("random-years-any", f.years.is_none(), accent)
                .on_click(cx.listener(|root, _, _, cx| root.set_random(cx, |f| f.years = None)))
                .child("Любые")
                .into_any_element(),
        ];
        years.extend(random::YEARS.iter().enumerate().map(|(i, (value, label))| {
            let value = value.to_string();
            chip(
                ("random-years", i as u64),
                f.years.as_deref() == Some(&value),
                accent,
            )
            .on_click(cx.listener(move |root, _, _, cx| {
                let value = value.clone();
                root.set_random(cx, |f| f.years = Some(value))
            }))
            .child(*label)
            .into_any_element()
        }));
        let mut studios = vec![
            chip("random-studio-any", f.studio.is_none(), accent)
                .on_click(cx.listener(|root, _, _, cx| root.set_random(cx, |f| f.studio = None)))
                .child("Любая")
                .into_any_element(),
        ];
        studios.extend(random::STUDIOS.iter().map(|&(id, name)| {
            chip(("random-studio", id as u64), f.studio == Some(id), accent)
                .on_click(
                    cx.listener(move |root, _, _, cx| root.set_random(cx, |f| f.studio = Some(id))),
                )
                .child(name)
                .into_any_element()
        }));
        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .w_full()
            .child(row("Жанры", genres))
            .child(row("Годы", years))
            .child(row("Студия", studios))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .mt(px(6.))
                    .child(
                        button(
                            "random-reset",
                            "Сбросить",
                            None,
                            Variant::Glass,
                            Size::M,
                            accent,
                        )
                        .on_click(cx.listener(|root, _, _, cx| {
                            root.set_random(cx, |f| *f = RandomFilter::default())
                        })),
                    )
                    .child(
                        button(
                            "random-go",
                            "Найти",
                            Some("shuffle"),
                            Variant::Primary,
                            Size::M,
                            accent,
                        )
                        .on_click(cx.listener(|root, _, _, cx| {
                            root.random.settings = false;
                            root.roll_random(cx);
                        })),
                    ),
            )
            .into_any_element();
        let root = cx.entity().downgrade();
        modal(
            "random-settings",
            self.random.settings,
            "Случайный тайтл",
            body,
            Rc::new(move |_, cx| {
                root.update(cx, |root, cx| {
                    root.random.settings = false;
                    cx.notify();
                })
                .ok();
            }),
            window,
            cx,
        )
    }

    /// The lists that show the library fetch it again.
    pub fn library_changed(&mut self, cx: &mut Context<Self>) {
        self.home.update(cx, |home, cx| home.reload(false, cx));
        if let Some(page) = &self.library {
            page.update(cx, |page, cx| page.reload(cx));
        }
        if let Some(page) = &self.history_page {
            page.update(cx, |page, cx| page.reload(cx));
        }
        if let Some(page) = &self.stats {
            page.update(cx, |page, cx| page.reload(cx));
        }
        // A page switched between seasons is in the map once per season.
        let mut seen = std::collections::HashSet::new();
        for page in self.titles.values() {
            if seen.insert(page.entity_id()) {
                page.update(cx, |page, cx| page.reload_progress(cx));
            }
        }
    }

    /// Ctrl+K: the quick jump opens, or closes when it is open. Not over the player.
    pub fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.take().is_some() {
            window.focus(&self.focus, cx);
        } else if self.player.is_none() {
            let core = self.core.clone();
            let palette = cx.new(|cx| CommandPalette::new(core, window, cx));
            cx.subscribe_in(
                &palette,
                window,
                |root, _, event: &PaletteEvent, window, cx| {
                    root.palette = None;
                    window.focus(&root.focus, cx);
                    if let PaletteEvent::Go(route) = event {
                        root.open(*route, cx);
                    }
                    cx.notify();
                },
            )
            .detach();
            self.palette = Some(palette);
        }
        cx.notify();
    }

    /// A newer version, offered in the corner of the window.
    pub fn offer_update(&mut self, info: UpdateInfo, cx: &mut Context<Self>) {
        self.update = Some(info);
        cx.notify();
    }

    /// Offers the plugin repository a link named; nothing is installed until the user agrees.
    pub fn offer_repo(&mut self, repo: String, window: &mut Window, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let offer = cx.new(|cx| RepoOffer::new(core, repo, cx));
        cx.subscribe_in(&offer, window, |root, _, _: &OfferClosed, _, cx| {
            root.offer = None;
            cx.notify();
        })
        .detach();
        self.on_page_events(&offer, window, cx);
        self.offer = Some(offer);
        cx.notify();
    }

    /// The first start's steps, for `shot`.
    pub fn show_onboarding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.onboarding = Some(onboarding(&self.core, window, cx));
        cx.notify();
    }

    /// Opens the catalog searching `text`, for `shot`.
    pub fn search_for(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.open(Route::Catalog, cx);
        let page = self.catalog_page(window, cx);
        page.update(cx, |page, cx| page.set_query(text, cx));
    }

    fn library_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<LibraryPage> {
        if let Some(page) = &self.library {
            return page.clone();
        }
        let core = self.core.clone();
        let page = cx.new(|cx| LibraryPage::new(core, cx));
        cx.subscribe_in(&page, window, |root, _, Open(route), _, cx| {
            root.open(*route, cx)
        })
        .detach();
        self.on_page_events(&page, window, cx);
        self.library = Some(page.clone());
        page
    }

    fn catalog_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<CatalogPage> {
        if let Some(page) = &self.catalog {
            return page.clone();
        }
        let core = self.core.clone();
        let page = cx.new(|cx| CatalogPage::new(core, cx));
        self.on_page_events(&page, window, cx);
        self.catalog = Some(page.clone());
        page
    }

    /// The catalog of a genre or a studio, from a title's link.
    fn browse(&mut self, seed: Seed, window: &mut Window, cx: &mut Context<Self>) {
        let page = self.catalog_page(window, cx);
        page.update(cx, |page, cx| page.show(seed, cx));
        self.open(Route::Catalog, cx);
    }

    fn title_page(
        &mut self,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<Title> {
        if let Some(page) = self.titles.get(&id) {
            return page.clone();
        }
        let core = self.core.clone();
        let page = cx.new(|cx| Title::new(core, id, cx));
        cx.subscribe_in(
            &page,
            window,
            |root, page, event: &TitleEvent, window, cx| match event {
                TitleEvent::Season(id) => {
                    root.titles.insert(*id, page.clone());
                    root.glide.stop();
                    root.history.open_keeping_scroll(Route::Title(*id));
                    cx.notify();
                }
                TitleEvent::Open(route) => root.open(*route, cx),
                TitleEvent::Play(request) => root.play(request.clone(), window, cx),
                TitleEvent::Changed => root.library_changed(cx),
                TitleEvent::Browse(seed) => root.browse(seed.clone(), window, cx),
            },
        )
        .detach();
        self.titles.insert(id, page.clone());
        page
    }

    pub fn open(&mut self, route: Route, cx: &mut Context<Self>) {
        self.glide.stop();
        self.autoscroll.stop();
        // Search is the catalog's: going there is going to the catalog's field.
        self.focus_search = route == Route::Search;
        let route = if route == Route::Search {
            Route::Catalog
        } else {
            route
        };
        self.history.open(route);
        cx.notify();
    }

    /// Back leaves the player first.
    fn back(&mut self, cx: &mut Context<Self>) {
        match &self.player {
            Some(player) => player.update(cx, |p, cx| p.close(cx)),
            None if self.history.back() => {
                self.glide.stop();
                cx.notify()
            }
            None => {}
        }
    }

    fn forward(&mut self, cx: &mut Context<Self>) {
        if self.player.is_none() && self.history.forward() {
            self.glide.stop();
            cx.notify();
        }
    }

    fn play(&mut self, request: Request, window: &mut Window, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let player = cx.new(|cx| Player::new(core, request, window, cx));
        cx.subscribe_in(&player, window, |root, _, _: &Closed, window, cx| {
            root.player = None;
            // The core saves the progress as it stops; then the pages show it.
            let player = root.core.player.clone();
            let stopped = root.core.rt.spawn(async move { player.stop().await });
            cx.spawn_in(window, async move |root, cx| {
                let _ = stopped.await;
                root.update(cx, |root, cx| root.library_changed(cx)).ok();
            })
            .detach();
            window.focus(&root.focus, cx);
            cx.notify();
        })
        .detach();
        self.player = Some(player);
        cx.notify();
    }

    /// The player screen over `state`, nothing playing: for `shot`.
    pub fn preview_player(
        &mut self,
        request: Request,
        state: ninari_core::player::PlayerState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let core = self.core.clone();
        self.player = Some(cx.new(|cx| Player::preview(core, request, state, window, cx)));
        cx.notify();
    }

    /// What the core's player asks of the window.
    /// The player host's messages, passed on by the app (see main.rs).
    pub fn on_host(&mut self, msg: HostMsg, window: &mut Window, cx: &mut Context<Self>) {
        match msg {
            HostMsg::Event(event) => {
                if let Some(player) = &self.player {
                    player.update(cx, |p, cx| p.on_event(event, cx));
                }
            }
            HostMsg::Start(options, reply) => {
                let started = self
                    .start_video(&options, window)
                    .map_err(|e| AppError::new("Player", format!("{e:#}")));
                let _ = reply.send(started);
            }
            // The surface goes before the core it draws.
            HostMsg::Stop => {
                #[cfg(target_os = "linux")]
                {
                    self.video = None;
                }
            }
        }
    }

    /// libmpv with `options`, drawing into a surface under the window.
    #[cfg(target_os = "linux")]
    fn start_video(
        &mut self,
        options: &[(String, String)],
        window: &mut Window,
    ) -> anyhow::Result<()> {
        self.video = None;
        let log = std::env::var("NINARI_MPV_LOG").ok();
        let mute = std::env::var_os("NINARI_MUTE").is_some();
        let options: Vec<(&str, &str)> = options
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .chain(log.as_deref().map(|l| ("log-file", l)))
            // No sound device rather than mute: the player would save mute as the user's choice.
            .chain(mute.then_some(("ao", "null")))
            .collect();
        let mpv = Arc::new(Mpv::new(&options)?);
        let (w, h) = device_size(window);
        let video = Video::attach(&*window, &*window, mpv.clone(), w, h)?;
        self.video = Some((video, mpv));
        Ok(())
    }

    /// Never asked for: the player host only renders where it can (see `can_render`).
    #[cfg(not(target_os = "linux"))]
    fn start_video(&mut self, _: &[(String, String)], _: &mut Window) -> anyhow::Result<()> {
        anyhow::bail!("video inside the window is Linux only")
    }

    fn page(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match route {
            Route::Home => self.home.clone().into_any_element(),
            Route::Title(id) => self.title_page(id, window, cx).into_any_element(),
            Route::Library => self.library_page(window, cx).into_any_element(),
            Route::Catalog => {
                let page = self.catalog_page(window, cx);
                if std::mem::take(&mut self.focus_search) {
                    page.update(cx, |page, cx| page.focus_search(window, cx));
                }
                page.into_any_element()
            }
            Route::Character(id) => {
                if !self.people.contains_key(&route) {
                    let core = self.core.clone();
                    let page = cx.new(|cx| CharacterPage::new(core, id, cx));
                    self.on_page_events(&page, window, cx);
                    self.people.insert(route, page.into());
                }
                self.people[&route].clone().into_any_element()
            }
            Route::Person(id) => {
                if !self.people.contains_key(&route) {
                    let core = self.core.clone();
                    let page = cx.new(|cx| PersonPage::new(core, id, cx));
                    self.on_page_events(&page, window, cx);
                    self.people.insert(route, page.into());
                }
                self.people[&route].clone().into_any_element()
            }
            Route::Settings => {
                if self.settings.is_none() {
                    let core = self.core.clone();
                    let page = cx.new(|cx| SettingsPage::new(core, cx));
                    self.on_page_events(&page, window, cx);
                    self.settings = Some(page);
                }
                self.settings.clone().unwrap().into_any_element()
            }
            Route::Stats => {
                if self.stats.is_none() {
                    let core = self.core.clone();
                    let page = cx.new(|cx| StatsPage::new(core, cx));
                    self.on_page_events(&page, window, cx);
                    self.stats = Some(page);
                }
                self.stats.clone().unwrap().into_any_element()
            }
            Route::Schedule => {
                if self.schedule.is_none() {
                    let core = self.core.clone();
                    let page = cx.new(|cx| SchedulePage::new(core, cx));
                    self.on_page_events(&page, window, cx);
                    self.schedule = Some(page);
                }
                self.schedule.clone().unwrap().into_any_element()
            }
            Route::History => {
                if self.history_page.is_none() {
                    let core = self.core.clone();
                    let page = cx.new(|cx| HistoryPage::new(core, cx));
                    self.on_page_events(&page, window, cx);
                    self.history_page = Some(page);
                }
                self.history_page.clone().unwrap().into_any_element()
            }
            Route::Search => self.catalog_page(window, cx).into_any_element(),
        }
    }
}

/// The window's inside in device pixels, the size the video surface takes.
#[cfg(any(target_os = "linux", windows))]
fn device_size(window: &Window) -> (i32, i32) {
    let size = window.viewport_size();
    let scale = window.scale_factor();
    (
        (f32::from(size.width) * scale).round() as i32,
        (f32::from(size.height) * scale).round() as i32,
    )
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        popover::begin_frame(cx);
        let route = self.history.current().route;
        let hero = match route {
            Route::Home => self.home.read(cx).hero_card().cloned(),
            Route::Title(id) => {
                let page = self.title_page(id, window, cx);
                // Back or Forward onto another season of a page switched in place.
                if page.read(cx).shown() != id {
                    page.update(cx, |p, cx| p.show(id, cx));
                }
                page.read(cx).card().cloned()
            }
            _ => None,
        };
        let title = match route {
            Route::Title(id) if self.player.is_none() => self.titles.get(&id).cloned(),
            _ => None,
        };
        if let Some(old) = self
            .drawn_title
            .take_if(|old| Some(&*old) != title.as_ref())
        {
            old.update(cx, |page, _| page.release(window));
        }
        self.drawn_title = title;
        let palette = match (&self.player, hero) {
            (Some(player), _) => player.read(cx).palette().clone(),
            (None, Some(card)) => poster(&card, cx).map(|(_, p)| p).unwrap_or_default(),
            _ => Palette::default(),
        };
        Accent::follow(&palette, window, cx);

        let root = div()
            .track_focus(&self.focus)
            .size_full()
            .font_family(theme::FONT)
            .text_color(rgb(theme::TEXT))
            // Escape closes the topmost menu or dialog before anything else hears it.
            .capture_key_down(cx.listener(|root, e: &KeyDownEvent, window, cx| {
                let k = &e.keystroke;
                if k.key == "escape" && popover::escape(window, cx) {
                    cx.stop_propagation();
                } else if k.key == "k" && (k.modifiers.control || k.modifiers.platform) {
                    root.toggle_palette(window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|root, _: &Back, _, cx| root.back(cx)))
            .on_action(cx.listener(|root, _: &Forward, _, cx| root.forward(cx)))
            .on_mouse_up(
                MouseButton::Navigate(NavigationDirection::Back),
                cx.listener(|root, _, _, cx| root.back(cx)),
            )
            .on_mouse_up(
                MouseButton::Navigate(NavigationDirection::Forward),
                cx.listener(|root, _, _, cx| root.forward(cx)),
            );
        #[cfg(target_os = "linux")]
        if let Some((video, _)) = &self.video {
            let (w, h) = device_size(window);
            video.resize(w, h);
        }
        #[cfg(windows)]
        if let Some(video) = &mut self.video_window {
            match &self.player {
                Some(_) => {
                    let (w, h) = device_size(window);
                    video.show(w, h);
                }
                None => video.hide(),
            }
        }
        if let Some(player) = &self.player {
            return image_cache(self.images.clone())
                .size_full()
                .child(root.child(player.clone()));
        }

        // The poster's glow behind the top of every page. GPUI rasterises an SVG at its full size
        // on the frame it changes size: the soft gradient is drawn an eighth of the size and
        // stretched, which looks the same.
        const SHRINK: f32 = 8.;
        let view = window.viewport_size();
        let left = px(rail::WIDTH) - view.width * 0.1;
        let (top, w, h) = (
            -view.height * 0.2,
            view.width * 1.1 - left,
            view.height * 0.7,
        );
        let glow = svg()
            .path("glow.svg")
            .absolute()
            .left(left + (w - w / SHRINK) / 2.)
            .top(top + (h - h / SHRINK) / 2.)
            .w(w / SHRINK)
            .h(h / SHRINK)
            .with_transformation(Transformation::scale(size(SHRINK, SHRINK)))
            .text_color(Accent::colors(cx).1.opacity(0.85));

        let scroll = self.history.current().scroll.clone();
        self.glide.step(&scroll, window);
        // A scroll moves the cards from under a still pointer: their details go.
        let offset = scroll.offset().y;
        if self
            .last_offset
            .replace(offset)
            .is_some_and(|last| last != offset)
        {
            hover_card::hide(cx);
        }
        self.autoscroll.step(&scroll, window);
        let page = self.page(route, window, cx);
        let random_bar = self.random_bar(route, cx);
        let random_settings = self.random_settings(window, cx);
        let details = {
            let (open, changed) = (cx.entity().downgrade(), cx.entity().downgrade());
            hover_card::overlay(
                hover_card::Actions {
                    open: Rc::new(move |id, _, cx| {
                        open.update(cx, |root, cx| root.open(Route::Title(id), cx))
                            .ok();
                    }),
                    changed: Rc::new(move |_, cx| {
                        changed.update(cx, |root, cx| root.library_changed(cx)).ok();
                    }),
                },
                window,
                cx,
            )
        };
        // The page's controls along the top once it has scrolled past them.
        let pinned = match route {
            Route::Catalog => self
                .catalog
                .clone()
                .and_then(|p| p.update(cx, |p, cx| p.pinned(window, cx))),
            Route::Library => self
                .library
                .clone()
                .and_then(|p| p.update(cx, |p, cx| p.pinned(window, cx))),
            _ => None,
        };
        let root = root
            .relative()
            .flex()
            .bg(rgb(theme::INK))
            .child(glow)
            .child(rail::rail(
                route,
                Accent::colors(cx).0,
                |to| Box::new(cx.listener(move |root, _, _, cx| root.open(to, cx))),
                Box::new(cx.listener(|root, _, _, cx| root.roll_random(cx))),
                self.random.rolling,
            ))
            .child(
                div()
                    .relative()
                    .flex_1()
                    // Rows of cards are wider than the window; they scroll, the page does not grow.
                    .min_w(px(0.))
                    .h_full()
                    .child(
                        div()
                            .id("page")
                            .size_full()
                            .overflow_y_scroll()
                            .restrict_scroll_to_axis()
                            .track_scroll(&scroll)
                            .child(page),
                    )
                    .child(self.glide.catcher(scroll.clone()))
                    .child(self.autoscroll.layer())
                    .children(pinned)
                    .children(random_bar),
            )
            .children(self.autoscroll.mark())
            .children(random_settings)
            .children(details)
            .children(self.palette.clone())
            .children(self.offer.clone())
            .children(self.update.clone().map(|info| update_banner(info, cx)))
            .children(self.onboarding.clone());
        image_cache(self.images.clone()).size_full().child(root)
    }
}

/// A new version in the corner of the window: offered, then downloading and installing in place.
fn update_banner(info: UpdateInfo, cx: &mut Context<Root>) -> impl IntoElement + use<> {
    let accent = Accent::colors(cx).0;
    let progress = updates::progress(cx);
    let (title, busy) = match &progress {
        Progress::Downloading(share) => (
            format!("Скачиваем {} · {}%", info.version, (share * 100.).round()),
            true,
        ),
        Progress::Installing => (format!("Устанавливаем {}", info.version), true),
        _ => (format!("Доступна версия {}", info.version), false),
    };
    let installs = updates::can_install(&info);
    let (share, failed) = match progress {
        Progress::Downloading(share) => (Some(share), None),
        Progress::Installing => (Some(1.), None),
        Progress::Failed(e) => (None, Some(e)),
        Progress::Idle => (None, None),
    };
    let get = {
        let info = info.clone();
        button(
            "update-get",
            match (installs, failed.is_some()) {
                (true, true) => "Повторить",
                (true, false) => "Обновить",
                (false, _) => "Скачать",
            },
            None,
            Variant::Primary,
            Size::M,
            accent,
        )
        .on_click(move |_, _, cx| updates::install(&info, cx))
    };
    div()
        .occlude()
        .absolute()
        .right(px(24.))
        .bottom(px(24.))
        .w(px(320.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(16.))
        .rounded(theme::R_L)
        .bg(rgba(0x12121af0))
        .border_1()
        .border_color(rgba(0xffffff14))
        .child(div().font_weight(FontWeight::EXTRA_BOLD).child(title))
        .children(share.map(|share| {
            div().h(px(4.)).rounded_full().bg(rgba(0xffffff14)).child(
                div()
                    .h_full()
                    .rounded_full()
                    .bg(accent)
                    .w(relative(share.clamp(0.02, 1.))),
            )
        }))
        .children(info.notes.filter(|_| !busy).map(|n| {
            div()
                .text_size(px(13.))
                .text_color(rgb(theme::TEXT_2))
                .line_clamp(4)
                .text_ellipsis()
                .child(n)
        }))
        .children(failed.map(|e| {
            div()
                .text_size(px(13.))
                .text_color(rgb(theme::DANGER))
                .line_clamp(3)
                .text_ellipsis()
                .child(format!("Не удалось обновить: {e}"))
        }))
        .when(!busy, |banner| {
            banner.child(
                div().flex().gap(px(8.)).mt(px(4.)).child(get).child(
                    button(
                        "update-later",
                        "Позже",
                        None,
                        Variant::Glass,
                        Size::M,
                        accent,
                    )
                    .on_click(cx.listener(|root, _, _, cx| {
                        root.update = None;
                        cx.notify();
                    })),
                ),
            )
        })
}

fn onboarding(core: &Arc<Core>, window: &mut Window, cx: &mut Context<Root>) -> Entity<Onboarding> {
    let core = core.clone();
    let view = cx.new(|cx| Onboarding::new(core, cx));
    cx.subscribe_in(&view, window, |root, _, _: &Done, _, cx| {
        root.onboarding = None;
        cx.notify();
    })
    .detach();
    cx.subscribe_in(&view, window, |root, _, event: &PageEvent, _, cx| {
        if matches!(event, PageEvent::Changed) {
            root.library_changed(cx);
        }
    })
    .detach();
    view
}
