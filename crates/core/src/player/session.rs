//! Playback bookkeeping for the episode currently in mpv, kept free of I/O so it can be tested.

use crate::library::{is_watched, EpisodeProgress};

const SAVE_EVERY: f64 = 5.0;
const PREFETCH_BEFORE_END: f64 = 120.0;
/// A skip starting this close to the end of an opening or ending would jump for nothing.
const SKIP_MARGIN: f64 = 2.0;

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Save,
    Prefetch(i64),
    PlayNext(i64),
    Finished,
    /// Jump over an opening or an ending; `from` lets the user go back to it.
    Skip {
        part: &'static str,
        from: f64,
        to: f64,
    },
}

#[derive(Debug, Clone)]
pub struct Playback {
    pub shikimori_id: i64,
    pub episode: i64,
    pub dub_key: String,
    pub extension_id: String,
    pub episodes_available: i64,
    pub final_episode: Option<i64>,
    pub ending_start: Option<f64>,
    pub position: f64,
    pub duration: f64,
    pub video_height: u32,
    pub paused: bool,
    pub buffering: bool,
    pub volume: f64,
    pub muted: bool,
    /// Playback time up to which mpv has the stream cached.
    pub cached_until: f64,
    last_saved_at: f64,
    prefetch_requested: bool,
    last_emit_ms: u64,
    dirty: bool,
    /// Openings and endings to skip, each once per episode.
    auto_opening: Option<(f64, f64)>,
    auto_ending: Option<(f64, f64)>,
    skipped_opening: bool,
    skipped_ending: bool,
    /// Fillers and recaps that going on by itself passes over; empty unless the user asked for it.
    skip_episodes: std::collections::BTreeSet<i64>,
}

fn skip_in(
    range: Option<(f64, f64)>,
    done: &mut bool,
    pos: f64,
    part: &'static str,
) -> Option<Action> {
    let (from, to) = range?;
    if *done || pos < from || pos >= to - SKIP_MARGIN {
        return None;
    }
    *done = true;
    Some(Action::Skip { part, from, to })
}

impl Playback {
    pub fn new(
        shikimori_id: i64,
        episode: i64,
        dub_key: String,
        extension_id: String,
        episodes_available: i64,
        final_episode: Option<i64>,
        ending_start: Option<f64>,
    ) -> Self {
        Self {
            shikimori_id,
            episode,
            dub_key,
            extension_id,
            episodes_available,
            final_episode,
            ending_start,
            position: 0.0,
            duration: 0.0,
            video_height: 0,
            paused: false,
            buffering: true,
            volume: 100.0,
            muted: false,
            cached_until: 0.0,
            last_saved_at: 0.0,
            prefetch_requested: false,
            last_emit_ms: 0,
            dirty: true,
            auto_opening: None,
            auto_ending: None,
            skipped_opening: false,
            skipped_ending: false,
            skip_episodes: Default::default(),
        }
    }

    /// Openings and endings to skip; `None` plays them.
    pub fn auto_skip(mut self, opening: Option<(f64, f64)>, ending: Option<(f64, f64)>) -> Self {
        self.set_auto_skip(opening, ending);
        self
    }

    pub fn set_auto_skip(&mut self, opening: Option<(f64, f64)>, ending: Option<(f64, f64)>) {
        self.auto_opening = opening;
        self.auto_ending = ending;
    }

    /// Starts bookkeeping from a resume position, so an early save cannot overwrite it with 0.
    pub fn resume_at(mut self, position: f64) -> Self {
        self.position = position;
        self.last_saved_at = position;
        self
    }

    /// The episode that comes on by itself after this one.
    pub fn next(&self) -> Option<i64> {
        (self.episode + 1..=self.episodes_available).find(|n| !self.skip_episodes.contains(n))
    }

    /// The episode after this one in order, for the "next episode" button.
    pub fn next_in_order(&self) -> Option<i64> {
        (self.episode < self.episodes_available).then_some(self.episode + 1)
    }

    pub fn set_skip_episodes(&mut self, episodes: std::collections::BTreeSet<i64>) {
        self.skip_episodes = episodes;
    }

    pub fn prev(&self) -> Option<i64> {
        (self.episode > 1).then_some(self.episode - 1)
    }

    /// Something besides the playback time changed; the UI should hear about it at once.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Time updates reach the UI at most every 200 ms; other changes go out immediately.
    pub fn should_emit(&mut self, now_ms: u64) -> bool {
        if self.dirty || now_ms.saturating_sub(self.last_emit_ms) >= 200 || self.last_emit_ms == 0 {
            self.dirty = false;
            self.last_emit_ms = now_ms;
            return true;
        }
        false
    }

    pub fn on_time(&mut self, pos: f64) -> Vec<Action> {
        self.position = pos;
        let mut out = Vec::new();
        if (pos - self.last_saved_at).abs() >= SAVE_EVERY {
            self.last_saved_at = pos;
            out.push(Action::Save);
        }
        if !self.prefetch_requested
            && self.duration > 0.0
            && self.duration - pos <= PREFETCH_BEFORE_END
        {
            if let Some(next) = self.next() {
                self.prefetch_requested = true;
                out.push(Action::Prefetch(next));
            }
        }
        out.extend(skip_in(
            self.auto_opening,
            &mut self.skipped_opening,
            pos,
            "opening",
        ));
        out.extend(skip_in(
            self.auto_ending,
            &mut self.skipped_ending,
            pos,
            "ending",
        ));
        out
    }

    /// `reason` is mpv's end-file reason: "eof" when the episode played to the end.
    pub fn on_end(&mut self, reason: &str) -> Vec<Action> {
        if reason != "eof" {
            return vec![Action::Save];
        }
        self.position = self.position.max(self.duration);
        match self.next() {
            Some(n) => vec![Action::Save, Action::PlayNext(n)],
            None => vec![Action::Save, Action::Finished],
        }
    }

    pub fn progress(&self, updated_at: i64) -> EpisodeProgress {
        EpisodeProgress {
            shikimori_id: self.shikimori_id,
            episode: self.episode,
            position_sec: self.position,
            duration_sec: self.duration,
            watched: is_watched(self.position, self.duration, self.ending_start),
            dub_key: Some(self.dub_key.clone()),
            extension_id: Some(self.extension_id.clone()),
            updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pb(episode: i64, available: i64) -> Playback {
        let mut p = Playback::new(
            1,
            episode,
            "anidub".into(),
            "anilibria".into(),
            available,
            Some(12),
            None,
        );
        p.duration = 1440.0;
        p
    }

    #[test]
    fn saves_every_five_seconds_of_playback() {
        let mut p = pb(1, 12);
        assert!(p.on_time(1.0).is_empty());
        assert!(p.on_time(4.9).is_empty());
        assert_eq!(p.on_time(5.1), vec![Action::Save]);
        assert!(p.on_time(6.0).is_empty());
        assert_eq!(p.on_time(10.3), vec![Action::Save]);
        assert_eq!(p.on_time(300.0), vec![Action::Save], "a seek also saves");
    }

    #[test]
    fn prefetches_next_episode_once_two_minutes_before_end() {
        let mut p = pb(3, 12);
        assert!(!p.on_time(1300.0).contains(&Action::Prefetch(4)));
        assert!(p.on_time(1321.0).contains(&Action::Prefetch(4)));
        assert!(!p.on_time(1330.0).contains(&Action::Prefetch(4)));
    }

    #[test]
    fn no_prefetch_on_last_available_episode() {
        let mut p = pb(12, 12);
        assert!(!p
            .on_time(1400.0)
            .iter()
            .any(|a| matches!(a, Action::Prefetch(_))));
    }

    #[test]
    fn going_on_by_itself_passes_over_episodes_to_skip() {
        let mut p = pb(3, 10);
        p.set_skip_episodes([4, 5, 7].into_iter().collect());
        assert_eq!(p.next(), Some(6));
        assert_eq!(p.on_end("eof"), vec![Action::Save, Action::PlayNext(6)]);
        let mut last = pb(8, 10);
        last.set_skip_episodes([9, 10].into_iter().collect());
        assert_eq!(
            last.on_end("eof"),
            vec![Action::Save, Action::Finished],
            "only fillers left: done"
        );
    }

    #[test]
    fn eof_saves_as_watched_and_plays_next() {
        let mut p = pb(3, 12);
        p.on_time(1000.0);
        assert_eq!(p.on_end("eof"), vec![Action::Save, Action::PlayNext(4)]);
        assert!(p.progress(0).watched);
        assert_eq!(p.progress(0).position_sec, 1440.0);
    }

    #[test]
    fn eof_on_last_episode_finishes() {
        let mut p = pb(12, 12);
        assert_eq!(p.on_end("eof"), vec![Action::Save, Action::Finished]);
    }

    #[test]
    fn resumed_session_keeps_start_position_until_time_updates() {
        let mut p = pb(5, 12).resume_at(600.0);
        assert_eq!(p.progress(0).position_sec, 600.0);
        assert!(p.on_time(602.0).is_empty(), "no save right after resuming");
        assert_eq!(p.on_end("quit"), vec![Action::Save]);
        assert_eq!(p.progress(0).position_sec, 602.0);
    }

    #[test]
    fn time_updates_are_throttled_for_the_ui() {
        let mut p = pb(1, 12);
        assert!(p.should_emit(1_000));
        assert!(!p.should_emit(1_100));
        assert!(p.should_emit(1_250));
        p.mark_dirty();
        assert!(
            p.should_emit(1_260),
            "state changes other than time are sent at once"
        );
    }

    #[test]
    fn neighbours_depend_on_available_episodes() {
        assert_eq!((pb(1, 12).prev(), pb(1, 12).next()), (None, Some(2)));
        assert_eq!((pb(12, 12).prev(), pb(12, 12).next()), (Some(11), None));
    }

    fn skips(actions: &[Action]) -> Vec<Action> {
        actions
            .iter()
            .filter(|a| matches!(a, Action::Skip { .. }))
            .cloned()
            .collect()
    }

    #[test]
    fn the_opening_is_skipped_once() {
        let mut p = pb(3, 12).auto_skip(Some((90.0, 180.0)), None);
        assert!(skips(&p.on_time(80.0)).is_empty());
        assert_eq!(
            skips(&p.on_time(91.0)),
            [Action::Skip {
                part: "opening",
                from: 90.0,
                to: 180.0
            }]
        );
        assert!(
            skips(&p.on_time(95.0)).is_empty(),
            "went back to watch it: no second skip"
        );
    }

    #[test]
    fn a_resume_inside_the_opening_skips_it_too() {
        let mut p = pb(3, 12)
            .resume_at(100.0)
            .auto_skip(Some((90.0, 180.0)), None);
        assert_eq!(skips(&p.on_time(100.4)).len(), 1);
    }

    #[test]
    fn the_ending_is_skipped_to_its_end() {
        let mut p = pb(3, 12).auto_skip(None, Some((1300.0, 1440.0)));
        assert_eq!(
            skips(&p.on_time(1301.0)),
            [Action::Skip {
                part: "ending",
                from: 1300.0,
                to: 1440.0
            }]
        );
    }

    #[test]
    fn nothing_is_skipped_at_the_very_end_of_a_range_or_when_off() {
        let mut p = pb(3, 12).auto_skip(Some((90.0, 180.0)), None);
        assert!(skips(&p.on_time(179.0)).is_empty());
        let mut off = pb(3, 12);
        assert!(skips(&off.on_time(100.0)).is_empty());
    }

    #[test]
    fn closing_the_player_only_saves() {
        let mut p = pb(3, 12);
        p.on_time(600.0);
        assert_eq!(p.on_end("quit"), vec![Action::Save]);
        assert!(!p.progress(0).watched);
        assert_eq!(p.progress(0).position_sec, 600.0);
    }
}
