//! The episode grid's tiles.

use ninari_core::catalog::jikan::EpisodeMark;
use ninari_core::library::EpisodeProgress;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TileState {
    Watched,
    Current,
    Available,
    /// Out, but not in the chosen dub.
    Missing,
    /// Not aired yet.
    Upcoming,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tile {
    pub n: i64,
    pub state: TileState,
    /// Share of the episode already watched, 0..=1.
    pub fraction: f64,
}

pub fn episode_tiles(
    total: i64,
    aired: i64,
    in_dub: &[i64],
    progress: &[EpisodeProgress],
    continue_episode: Option<i64>,
) -> Vec<Tile> {
    let aired = aired.max(in_dub.iter().copied().max().unwrap_or(0));
    let total = total.max(aired);
    (1..=total)
        .map(|n| {
            let p = progress.iter().find(|p| p.episode == n);
            let fraction = match p {
                Some(p) if p.watched => 1.,
                Some(p) if p.duration_sec > 0. => (p.position_sec / p.duration_sec).min(1.),
                _ => 0.,
            };
            let state = if Some(n) == continue_episode {
                TileState::Current
            } else if p.is_some_and(|p| p.watched) {
                TileState::Watched
            } else if n > aired {
                TileState::Upcoming
            } else if in_dub.contains(&n) {
                TileState::Available
            } else {
                TileState::Missing
            };
            Tile { n, state, fraction }
        })
        .collect()
}

/// Blocks of 100 episodes for long titles; none when they fit at once.
pub fn ranges(total: i64) -> Vec<(i64, i64)> {
    if total <= 100 {
        return Vec::new();
    }
    (1..=total)
        .step_by(100)
        .map(|from| (from, (from + 99).min(total)))
        .collect()
}

/// "5. Charlotte · филлер": what MyAnimeList knows about an episode, for its tile's popup;
/// None when it knows nothing worth showing.
pub fn tile_hint(n: i64, mark: Option<&EpisodeMark>) -> Option<String> {
    let mark = mark?;
    let kind = match (mark.filler, mark.recap) {
        (true, _) => Some("филлер"),
        (_, true) => Some("пересказ"),
        _ => None,
    };
    match (&mark.title, kind) {
        (Some(t), Some(k)) => Some(format!("{n}. {t} · {k}")),
        (Some(t), None) => Some(format!("{n}. {t}")),
        (None, Some(k)) => Some(format!("{n} · {k}")),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prog(episode: i64, position_sec: f64, watched: bool) -> EpisodeProgress {
        EpisodeProgress {
            shikimori_id: 1,
            episode,
            position_sec,
            duration_sec: 1440.,
            watched,
            dub_key: None,
            extension_id: None,
            updated_at: episode,
        }
    }

    #[test]
    fn tiles_tell_watched_current_available_missing_and_upcoming() {
        let tiles = episode_tiles(
            6,
            5,
            &[1, 2, 3, 4],
            &[prog(1, 1440., true), prog(2, 600., false)],
            Some(2),
        );
        let states: Vec<_> = tiles.iter().map(|t| t.state).collect();
        use TileState::*;
        assert_eq!(
            states,
            [Watched, Current, Available, Available, Missing, Upcoming]
        );
        assert!((tiles[1].fraction - 600. / 1440.).abs() < 1e-9);
        assert_eq!(tiles[0].fraction, 1.);
    }

    #[test]
    fn total_falls_back_to_what_exists() {
        let tiles = episode_tiles(0, 0, &[1, 2, 3], &[], Some(1));
        assert_eq!(tiles.len(), 3);
        assert_eq!(tiles[0].state, TileState::Current);
    }

    #[test]
    fn long_titles_split_into_hundreds() {
        assert!(ranges(12).is_empty());
        assert_eq!(ranges(250), [(1, 100), (101, 200), (201, 250)]);
    }

    #[test]
    fn tile_hints_name_the_episode_and_its_kind() {
        let mark = |filler, title: Option<&str>| EpisodeMark {
            number: 5,
            title: title.map(Into::into),
            filler,
            recap: false,
        };
        assert_eq!(
            tile_hint(5, Some(&mark(false, Some("Charlotte")))).as_deref(),
            Some("5. Charlotte")
        );
        assert_eq!(
            tile_hint(5, Some(&mark(true, Some("Siege")))).as_deref(),
            Some("5. Siege · филлер")
        );
        assert_eq!(
            tile_hint(5, Some(&mark(true, None))).as_deref(),
            Some("5 · филлер")
        );
        assert_eq!(tile_hint(5, Some(&mark(false, None))), None);
        assert_eq!(tile_hint(5, None), None);
    }
}
