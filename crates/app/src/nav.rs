//! Where the window is: the page on screen and the pages behind and ahead of it, each with its own
//! scroll.

use gpui::{ScrollHandle, actions, point, px};

actions!(nav, [Back, Forward]);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Route {
    Home,
    Search,
    Catalog,
    Library,
    Schedule,
    Settings,
    History,
    Stats,
    /// A title's page, by its Shikimori id.
    Title(i64),
    Character(i64),
    Person(i64),
}

impl Route {
    /// `shot`'s names for the pages: "home", "search", ... and "title:<id>".
    pub fn parse(name: &str) -> Option<Route> {
        if let Some(id) = name.strip_prefix("title:") {
            return id.parse().ok().map(Route::Title);
        }
        if let Some(id) = name.strip_prefix("character:") {
            return id.parse().ok().map(Route::Character);
        }
        if let Some(id) = name.strip_prefix("person:") {
            return id.parse().ok().map(Route::Person);
        }
        Some(match name {
            "home" => Route::Home,
            "search" => Route::Search,
            "catalog" => Route::Catalog,
            "library" => Route::Library,
            "schedule" => Route::Schedule,
            "settings" => Route::Settings,
            "history" => Route::History,
            "stats" => Route::Stats,
            _ => return None,
        })
    }
}

pub struct Entry {
    pub route: Route,
    /// Kept with the page, so coming back finds it scrolled where it was left.
    pub scroll: ScrollHandle,
}

pub struct History {
    entries: Vec<Entry>,
    at: usize,
}

impl History {
    pub fn new(route: Route) -> Self {
        Self {
            entries: vec![Entry {
                route,
                scroll: ScrollHandle::new(),
            }],
            at: 0,
        }
    }

    pub fn current(&self) -> &Entry {
        &self.entries[self.at]
    }

    /// Opens `route` at the top, dropping the pages ahead. The page already on screen just
    /// scrolls back up.
    pub fn open(&mut self, route: Route) {
        if self.current().route == route {
            self.current().scroll.set_offset(point(px(0.), px(0.)));
            return;
        }
        self.entries.truncate(self.at + 1);
        self.entries.push(Entry {
            route,
            scroll: ScrollHandle::new(),
        });
        self.at += 1;
    }

    /// Opens `route` scrolled where the page on screen is: another season of the same title,
    /// shown in place.
    pub fn open_keeping_scroll(&mut self, route: Route) {
        let offset = self.current().scroll.offset();
        self.open(route);
        self.current().scroll.set_offset(offset);
    }

    /// False when there is nothing behind.
    pub fn back(&mut self) -> bool {
        let moved = self.at > 0;
        self.at = self.at.saturating_sub(1);
        moved
    }

    /// False when there is nothing ahead.
    pub fn forward(&mut self) -> bool {
        let moved = self.at + 1 < self.entries.len();
        if moved {
            self.at += 1;
        }
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_back_and_forward_and_a_new_page_drops_the_ones_ahead() {
        let mut h = History::new(Route::Home);
        assert!(!h.back());
        h.open(Route::Search);
        h.open(Route::Library);
        assert!(h.back());
        assert_eq!(h.current().route, Route::Search);
        assert!(h.forward());
        assert_eq!(h.current().route, Route::Library);
        assert!(!h.forward());

        h.back();
        h.back();
        h.open(Route::Schedule);
        assert_eq!(h.current().route, Route::Schedule);
        assert!(!h.forward());
        assert!(h.back());
        assert_eq!(h.current().route, Route::Home);
    }

    #[test]
    fn a_season_switch_keeps_the_scroll() {
        let mut h = History::new(Route::Title(1));
        h.current().scroll.set_offset(point(px(0.), px(-640.)));
        h.open_keeping_scroll(Route::Title(2));
        assert_eq!(h.current().route, Route::Title(2));
        assert_eq!(h.current().scroll.offset().y, px(-640.));
        assert!(h.back());
        assert_eq!(h.current().route, Route::Title(1));
    }

    #[test]
    fn opening_the_page_on_screen_adds_nothing() {
        let mut h = History::new(Route::Home);
        h.open(Route::Home);
        assert!(!h.back());
    }

    #[test]
    fn pages_keep_their_own_scroll() {
        let mut h = History::new(Route::Home);
        h.current().scroll.set_offset(point(px(0.), px(-300.)));
        h.open(Route::Search);
        assert_eq!(h.current().scroll.offset().y, px(0.));
        h.back();
        assert_eq!(h.current().scroll.offset().y, px(-300.));
    }
}

/// What a page asks of the window.
#[derive(Clone)]
pub enum PageEvent {
    Open(Route),
    Play(crate::player::Request),
    /// The library changed: lists elsewhere need a reload.
    Changed,
}
