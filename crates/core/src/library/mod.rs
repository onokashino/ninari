//! The user's list and viewing progress: what counts as watched and where to continue.

use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::db::{now_ts, Db};
use crate::error::AppError;

/// Titles with this status only remember preferences (the chosen dub) and are not shown in lists.
const HIDDEN: &str = "none";
const STATUSES: [&str; 5] = ["watching", "planned", "completed", "dropped", "on_hold"];
const WATCHED_SHARE: f64 = 0.9;
const RESUME_REWIND: f64 = 5.0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeProgress {
    pub shikimori_id: i64,
    pub episode: i64,
    pub position_sec: f64,
    pub duration_sec: f64,
    pub watched: bool,
    pub dub_key: Option<String>,
    pub extension_id: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryEntry {
    pub shikimori_id: i64,
    pub status: String,
    pub chosen_dub_key: Option<String>,
    pub notify: bool,
    pub added_at: i64,
    pub updated_at: i64,
    /// The user's own score, 1-10. Absent in rows from versions before scores.
    #[serde(default)]
    pub score: Option<i64>,
    #[serde(default)]
    pub note: Option<String>,
}

const MAX_NOTE: usize = 4000;

pub fn is_watched(position: f64, duration: f64, ending_start: Option<f64>) -> bool {
    if duration <= 0.0 {
        return false;
    }
    let reached_ending = ending_start.is_some_and(|e| e > 0.0 && position >= e);
    reached_ending || position >= duration * WATCHED_SHARE
}

/// When the title was last watched: its latest progress update.
pub fn last_watched(progress: &[EpisodeProgress]) -> Option<i64> {
    progress.iter().map(|p| p.updated_at).max()
}

/// Episodes that came out after the last one the user opened; 0 when nothing was opened yet.
pub fn fresh_episodes(progress: &[EpisodeProgress], aired: i64) -> i64 {
    progress
        .iter()
        .map(|p| p.episode)
        .max()
        .map_or(0, |last| (aired - last).max(0))
}

/// Where a chosen episode starts: a few seconds before where it was left, from the start
/// when it was watched or barely opened.
pub fn resume_position(p: Option<&EpisodeProgress>) -> f64 {
    match p {
        Some(p) if !p.watched && p.position_sec > 2.0 * RESUME_REWIND => {
            p.position_sec - RESUME_REWIND
        }
        _ => 0.0,
    }
}

/// Where "Continue" should start: the last opened episode if unfinished (a few seconds back),
/// otherwise the next one from the start. `None` when everything available is watched.
pub fn continue_target(
    progress: &[EpisodeProgress],
    episodes_available: i64,
) -> Option<(i64, f64)> {
    let Some(last) = progress.iter().max_by_key(|p| (p.updated_at, p.episode)) else {
        return (episodes_available >= 1).then_some((1, 0.0));
    };
    if !last.watched {
        return Some((last.episode, (last.position_sec - RESUME_REWIND).max(0.0)));
    }
    let next = last.episode + 1;
    (next <= episodes_available).then_some((next, 0.0))
}

fn progress_from_row(r: &Row<'_>) -> rusqlite::Result<EpisodeProgress> {
    Ok(EpisodeProgress {
        shikimori_id: r.get(0)?,
        episode: r.get(1)?,
        position_sec: r.get(2)?,
        duration_sec: r.get(3)?,
        watched: r.get::<_, i64>(4)? != 0,
        dub_key: r.get(5)?,
        extension_id: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

const PROGRESS_COLUMNS: &str =
    "shikimori_id, episode, position_sec, duration_sec, watched, dub_key, extension_id, updated_at";

fn entry_from_row(r: &Row<'_>) -> rusqlite::Result<LibraryEntry> {
    Ok(LibraryEntry {
        shikimori_id: r.get(0)?,
        status: r.get(1)?,
        chosen_dub_key: r.get(2)?,
        notify: r.get::<_, i64>(3)? != 0,
        added_at: r.get(4)?,
        updated_at: r.get(5)?,
        score: r.get(6)?,
        note: r.get(7)?,
    })
}

const ENTRY_COLUMNS: &str =
    "shikimori_id, status, chosen_dub_key, notify, added_at, updated_at, score, note";

const BACKUP_VERSION: u32 = 1;

/// Rows another device changed, to be merged here: the newer edit of a row wins, and an episode
/// once watched stays watched.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    pub entries: Vec<LibraryEntry>,
    pub progress: Vec<EpisodeProgress>,
}

/// Everything the user made here: the list, remembered dubs and every progress row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    pub version: u32,
    pub exported_at: i64,
    pub library: Vec<LibraryEntry>,
    pub progress: Vec<EpisodeProgress>,
}

pub struct Library {
    db: Db,
}

impl Library {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    fn upsert_status(&self, id: i64, status: &str) -> Result<(), AppError> {
        let now = now_ts();
        self.db.lock().unwrap().execute(
            "INSERT INTO library (shikimori_id, status, added_at, updated_at) VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(shikimori_id) DO UPDATE SET status = excluded.status, updated_at = excluded.updated_at",
            params![id, status, now],
        )?;
        Ok(())
    }

    pub fn set_status(&self, id: i64, status: &str) -> Result<(), AppError> {
        if !STATUSES.contains(&status) {
            return Err(AppError::new(
                "BadRequest",
                format!("неизвестный статус {status}"),
            ));
        }
        self.upsert_status(id, status)
    }

    /// Adds a title from another list with its status and the first `episodes` marked watched.
    /// A title the library already has is left alone and `false` is returned; progress made
    /// here on a hidden title is kept.
    pub fn import_rate(
        &self,
        id: i64,
        status: &str,
        episodes: i64,
        updated_at: i64,
    ) -> Result<bool, AppError> {
        if self.get_entry(id)?.is_some_and(|e| e.status != HIDDEN) {
            return Ok(false);
        }
        let mut conn = self.db.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO library (shikimori_id, status, added_at, updated_at) VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(shikimori_id) DO UPDATE SET status = excluded.status, updated_at = excluded.updated_at",
            params![id, status, updated_at.max(1)],
        )?;
        for episode in 1..=episodes {
            tx.execute(
                "INSERT OR IGNORE INTO episode_progress (shikimori_id, episode, position_sec, duration_sec, watched, updated_at)
                 VALUES (?1, ?2, 0, 0, 1, ?3)",
                params![id, episode, updated_at],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn remove(&self, id: i64) -> Result<(), AppError> {
        self.upsert_status(id, HIDDEN)
    }

    pub fn set_dub(&self, id: i64, dub_key: &str) -> Result<(), AppError> {
        let now = now_ts();
        self.db.lock().unwrap().execute(
            "INSERT INTO library (shikimori_id, status, chosen_dub_key, added_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(shikimori_id) DO UPDATE SET chosen_dub_key = excluded.chosen_dub_key, updated_at = excluded.updated_at",
            params![id, HIDDEN, dub_key, now],
        )?;
        Ok(())
    }

    /// The user's score (1-10) and note; `None` clears them. A title not in the list keeps them hidden.
    pub fn set_rating(
        &self,
        id: i64,
        score: Option<i64>,
        note: Option<&str>,
    ) -> Result<(), AppError> {
        if score.is_some_and(|s| !(1..=10).contains(&s)) {
            return Err(AppError::new("BadRequest", "оценка от 1 до 10"));
        }
        let note = note.map(str::trim).filter(|n| !n.is_empty());
        if note.is_some_and(|n| n.chars().count() > MAX_NOTE) {
            return Err(AppError::new(
                "BadRequest",
                format!("заметка длиннее {MAX_NOTE} символов"),
            ));
        }
        let now = now_ts();
        self.db.lock().unwrap().execute(
            "INSERT INTO library (shikimori_id, status, score, note, added_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(shikimori_id) DO UPDATE SET score = excluded.score, note = excluded.note, updated_at = excluded.updated_at",
            params![id, HIDDEN, score, note, now],
        )?;
        Ok(())
    }

    pub fn get_entry(&self, id: i64) -> Result<Option<LibraryEntry>, AppError> {
        let conn = self.db.lock().unwrap();
        Ok(conn
            .query_row(
                &format!("SELECT {ENTRY_COLUMNS} FROM library WHERE shikimori_id = ?1"),
                [id],
                entry_from_row,
            )
            .optional()?)
    }

    pub fn list(&self, status: Option<&str>) -> Result<Vec<LibraryEntry>, AppError> {
        let conn = self.db.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {ENTRY_COLUMNS} FROM library WHERE status != ?1 AND (?2 IS NULL OR status = ?2) ORDER BY updated_at DESC"
        ))?;
        let rows = stmt
            .query_map(params![HIDDEN, status], entry_from_row)?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// Stores a progress tick. `ending_start` is the ending timing when the source knows it;
    /// `final_episode` is the last episode of a finished title, which completes it once watched.
    pub fn save_progress(
        &self,
        p: &EpisodeProgress,
        ending_start: Option<f64>,
        final_episode: Option<i64>,
    ) -> Result<(), AppError> {
        let watched = p.watched || is_watched(p.position_sec, p.duration_sec, ending_start);
        self.db.lock().unwrap().execute(
            &format!(
                "INSERT INTO episode_progress ({PROGRESS_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(shikimori_id, episode) DO UPDATE SET position_sec = excluded.position_sec,
                   duration_sec = CASE WHEN excluded.duration_sec > 0 THEN excluded.duration_sec ELSE duration_sec END,
                   watched = MAX(watched, excluded.watched),
                   dub_key = COALESCE(excluded.dub_key, dub_key), extension_id = COALESCE(excluded.extension_id, extension_id),
                   updated_at = excluded.updated_at"
            ),
            params![
                p.shikimori_id,
                p.episode,
                p.position_sec,
                p.duration_sec,
                watched as i64,
                p.dub_key,
                p.extension_id,
                p.updated_at
            ],
        )?;
        let status = self.get_entry(p.shikimori_id)?.map(|e| e.status);
        if watched && final_episode == Some(p.episode) {
            self.upsert_status(p.shikimori_id, "completed")?;
        } else if matches!(status.as_deref(), None | Some(HIDDEN) | Some("planned")) {
            self.upsert_status(p.shikimori_id, "watching")?;
        }
        Ok(())
    }

    pub fn progress(&self, id: i64) -> Result<Vec<EpisodeProgress>, AppError> {
        let conn = self.db.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {PROGRESS_COLUMNS} FROM episode_progress WHERE shikimori_id = ?1 ORDER BY episode"
        ))?;
        let rows = stmt
            .query_map([id], progress_from_row)?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// Every progress row, for statistics.
    pub fn all_progress(&self) -> Result<Vec<EpisodeProgress>, AppError> {
        let conn = self.db.lock().unwrap();
        let mut stmt = conn.prepare(&format!("SELECT {PROGRESS_COLUMNS} FROM episode_progress"))?;
        let rows = stmt
            .query_map([], progress_from_row)?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// The latest `limit` progress rows, newest first: the watch history.
    pub fn history(&self, limit: usize) -> Result<Vec<EpisodeProgress>, AppError> {
        let conn = self.db.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {PROGRESS_COLUMNS} FROM episode_progress ORDER BY updated_at DESC, episode DESC LIMIT ?1"
        ))?;
        let rows = stmt
            .query_map([limit as i64], progress_from_row)?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// Forgets one episode's progress and returns it, so it can be put back with `apply`.
    // A forgotten row is not sent to paired devices; they keep theirs until they forget it too.
    pub fn forget_episode(
        &self,
        id: i64,
        episode: i64,
    ) -> Result<Option<EpisodeProgress>, AppError> {
        let conn = self.db.lock().unwrap();
        let row = conn
            .query_row(
                &format!("DELETE FROM episode_progress WHERE shikimori_id = ?1 AND episode = ?2 RETURNING {PROGRESS_COLUMNS}"),
                [id, episode],
                progress_from_row,
            )
            .optional()?;
        Ok(row)
    }

    pub fn export(&self) -> Result<Backup, AppError> {
        let library = {
            let conn = self.db.lock().unwrap();
            let mut stmt = conn.prepare(&format!("SELECT {ENTRY_COLUMNS} FROM library"))?;
            let rows = stmt
                .query_map([], entry_from_row)?
                .collect::<Result<_, _>>()?;
            rows
        };
        Ok(Backup {
            version: BACKUP_VERSION,
            exported_at: now_ts(),
            library,
            progress: self.all_progress()?,
        })
    }

    /// Adds what a backup has and the library does not; nothing already here is overwritten,
    /// except a hidden title, which only remembered a dub. Returns (titles, episodes) added.
    pub fn restore(&self, backup: &Backup) -> Result<(usize, usize), AppError> {
        if backup.version != BACKUP_VERSION {
            return Err(AppError::new(
                "BadRequest",
                "копия сделана другой версией приложения",
            ));
        }
        let mut conn = self.db.lock().unwrap();
        let tx = conn.transaction()?;
        let mut titles = 0;
        for e in &backup.library {
            if e.status != HIDDEN && !STATUSES.contains(&e.status.as_str()) {
                continue;
            }
            let changed = tx.execute(
                &format!(
                    "INSERT INTO library ({ENTRY_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(shikimori_id) DO UPDATE SET status = excluded.status, notify = excluded.notify,
                       added_at = excluded.added_at, updated_at = excluded.updated_at,
                       chosen_dub_key = COALESCE(library.chosen_dub_key, excluded.chosen_dub_key),
                       score = COALESCE(library.score, excluded.score), note = COALESCE(library.note, excluded.note)
                     WHERE library.status = ?9 AND excluded.status != ?9"
                ),
                params![
                    e.shikimori_id,
                    e.status,
                    e.chosen_dub_key,
                    e.notify as i64,
                    e.added_at,
                    e.updated_at,
                    e.score,
                    e.note,
                    HIDDEN
                ],
            )?;
            if changed > 0 && e.status != HIDDEN {
                titles += 1;
            }
        }
        let mut episodes = 0;
        for p in &backup.progress {
            episodes += tx.execute(
                &format!("INSERT OR IGNORE INTO episode_progress ({PROGRESS_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"),
                params![
                    p.shikimori_id,
                    p.episode,
                    p.position_sec,
                    p.duration_sec,
                    p.watched as i64,
                    p.dub_key,
                    p.extension_id,
                    p.updated_at
                ],
            )?;
        }
        tx.commit()?;
        Ok((titles, episodes))
    }

    /// Library rows and progress rows changed after `since` (seconds), hidden titles included.
    pub fn changes_since(&self, since: i64) -> Result<Changes, AppError> {
        let conn = self.db.lock().unwrap();
        let entries = conn
            .prepare(&format!(
                "SELECT {ENTRY_COLUMNS} FROM library WHERE updated_at > ?1"
            ))?
            .query_map([since], entry_from_row)?
            .collect::<Result<_, _>>()?;
        let progress = conn
            .prepare(&format!(
                "SELECT {PROGRESS_COLUMNS} FROM episode_progress WHERE updated_at > ?1"
            ))?
            .query_map([since], progress_from_row)?
            .collect::<Result<_, _>>()?;
        Ok(Changes { entries, progress })
    }

    /// Merges another device's rows; returns how many (titles, episodes) changed here.
    pub fn apply(&self, changes: &Changes) -> Result<(usize, usize), AppError> {
        let mut conn = self.db.lock().unwrap();
        let tx = conn.transaction()?;
        let mut titles = 0;
        for e in &changes.entries {
            if e.status != HIDDEN && !STATUSES.contains(&e.status.as_str()) {
                continue;
            }
            titles += tx.execute(
                &format!(
                    "INSERT INTO library ({ENTRY_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(shikimori_id) DO UPDATE SET status = excluded.status,
                       chosen_dub_key = excluded.chosen_dub_key, notify = excluded.notify,
                       added_at = MIN(library.added_at, excluded.added_at), updated_at = excluded.updated_at,
                       score = excluded.score, note = excluded.note
                     WHERE excluded.updated_at > library.updated_at"
                ),
                // A device on a version before scores sends none, and its newer edit clears ours.
                params![
                    e.shikimori_id,
                    e.status,
                    e.chosen_dub_key,
                    e.notify as i64,
                    e.added_at,
                    e.updated_at,
                    e.score,
                    e.note
                ],
            )?;
        }
        let mut episodes = 0;
        for p in &changes.progress {
            episodes += tx.execute(
                &format!(
                    "INSERT INTO episode_progress ({PROGRESS_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(shikimori_id, episode) DO UPDATE SET
                       position_sec = CASE WHEN excluded.updated_at > episode_progress.updated_at
                         THEN excluded.position_sec ELSE episode_progress.position_sec END,
                       duration_sec = CASE WHEN excluded.updated_at > episode_progress.updated_at AND excluded.duration_sec > 0
                         THEN excluded.duration_sec ELSE episode_progress.duration_sec END,
                       dub_key = CASE WHEN excluded.updated_at > episode_progress.updated_at
                         THEN COALESCE(excluded.dub_key, episode_progress.dub_key) ELSE episode_progress.dub_key END,
                       extension_id = CASE WHEN excluded.updated_at > episode_progress.updated_at
                         THEN COALESCE(excluded.extension_id, episode_progress.extension_id) ELSE episode_progress.extension_id END,
                       watched = MAX(episode_progress.watched, excluded.watched),
                       updated_at = MAX(episode_progress.updated_at, excluded.updated_at)
                     WHERE excluded.updated_at > episode_progress.updated_at OR excluded.watched > episode_progress.watched"
                ),
                params![
                    p.shikimori_id,
                    p.episode,
                    p.position_sec,
                    p.duration_sec,
                    p.watched as i64,
                    p.dub_key,
                    p.extension_id,
                    p.updated_at
                ],
            )?;
        }
        tx.commit()?;
        Ok((titles, episodes))
    }

    /// The most recent progress row of every title with status "watching", newest first.
    pub fn continue_list(&self) -> Result<Vec<EpisodeProgress>, AppError> {
        let conn = self.db.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM episode_progress p
             JOIN library l ON l.shikimori_id = p.shikimori_id AND l.status = 'watching'
             WHERE p.episode = (SELECT q.episode FROM episode_progress q WHERE q.shikimori_id = p.shikimori_id
                                ORDER BY q.updated_at DESC, q.episode DESC LIMIT 1)
             ORDER BY p.updated_at DESC",
            PROGRESS_COLUMNS.split(", ").map(|c| format!("p.{c}")).collect::<Vec<_>>().join(", ")
        ))?;
        let rows = stmt
            .query_map([], progress_from_row)?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use std::sync::{Arc, Mutex};

    fn p(ep: i64, pos: f64, watched: bool) -> EpisodeProgress {
        EpisodeProgress {
            shikimori_id: 1,
            episode: ep,
            position_sec: pos,
            duration_sec: 1440.0,
            watched,
            dub_key: None,
            extension_id: None,
            updated_at: ep,
        }
    }

    fn lib() -> Library {
        Library::new(Arc::new(Mutex::new(db::open_in_memory().unwrap())))
    }

    fn entry_at(id: i64, status: &str, at: i64) -> LibraryEntry {
        LibraryEntry {
            shikimori_id: id,
            status: status.into(),
            chosen_dub_key: None,
            notify: true,
            added_at: at,
            updated_at: at,
            score: None,
            note: None,
        }
    }

    fn progress_at(ep: i64, pos: f64, watched: bool, at: i64) -> EpisodeProgress {
        EpisodeProgress {
            updated_at: at,
            ..p(ep, pos, watched)
        }
    }

    fn changes(entries: Vec<LibraryEntry>, progress: Vec<EpisodeProgress>) -> Changes {
        Changes { entries, progress }
    }

    #[test]
    fn a_score_and_note_stick_to_a_title_and_travel_with_it() {
        let l = lib();
        l.set_rating(1, Some(9), Some("  перепройти с субтитрами "))
            .unwrap();
        let e = l.get_entry(1).unwrap().unwrap();
        assert_eq!(
            (e.status.as_str(), e.score, e.note.as_deref()),
            ("none", Some(9), Some("перепройти с субтитрами"))
        );
        assert!(
            l.list(None).unwrap().is_empty(),
            "rating alone does not add to the list"
        );
        l.set_status(1, "completed").unwrap();
        assert_eq!(
            l.get_entry(1).unwrap().unwrap().score,
            Some(9),
            "the status keeps the score"
        );

        assert!(l.set_rating(1, Some(11), None).is_err());
        assert!(l
            .set_rating(1, None, Some(&"я".repeat(MAX_NOTE + 1)))
            .is_err());
        l.set_rating(1, None, Some("   ")).unwrap();
        let e = l.get_entry(1).unwrap().unwrap();
        assert_eq!((e.score, e.note), (None, None));

        let other = lib();
        let newer = LibraryEntry {
            score: Some(7),
            note: Some("с другого".into()),
            ..entry_at(1, "completed", e.updated_at + 1)
        };
        other.apply(&changes(vec![newer.clone()], vec![])).unwrap();
        assert_eq!(other.get_entry(1).unwrap().unwrap(), newer);
        let old: LibraryEntry = serde_json::from_str(
            r#"{"shikimoriId":2,"status":"planned","notify":true,"addedAt":1,"updatedAt":1}"#,
        )
        .unwrap();
        assert_eq!(old.score, None, "rows from before scores still read");
    }

    #[test]
    fn history_is_newest_first_and_a_forgotten_episode_can_come_back() {
        let l = lib();
        let rows = vec![
            progress_at(1, 600.0, true, 100),
            progress_at(2, 300.0, false, 300),
            progress_at(3, 900.0, false, 200),
        ];
        l.apply(&changes(vec![], rows)).unwrap();
        let eps = |l: &Library| {
            l.history(10)
                .unwrap()
                .iter()
                .map(|p| p.episode)
                .collect::<Vec<_>>()
        };
        assert_eq!(eps(&l), [2, 3, 1]);
        assert_eq!(l.history(1).unwrap().len(), 1);

        let gone = l.forget_episode(1, 1).unwrap().unwrap();
        assert!(gone.watched);
        assert_eq!(eps(&l), [2, 3]);
        assert_eq!(l.forget_episode(1, 1).unwrap(), None);
        l.apply(&changes(vec![], vec![gone.clone()])).unwrap();
        assert_eq!(l.progress(1).unwrap()[0], gone);
    }

    #[test]
    fn changes_since_a_moment_are_the_rows_touched_after_it() {
        let l = lib();
        l.apply(&changes(
            vec![entry_at(1, "watching", 100), entry_at(2, "planned", 300)],
            vec![
                progress_at(1, 600.0, false, 100),
                progress_at(2, 900.0, false, 300),
            ],
        ))
        .unwrap();
        let c = l.changes_since(200).unwrap();
        assert_eq!(
            c.entries.iter().map(|e| e.shikimori_id).collect::<Vec<_>>(),
            [2]
        );
        assert_eq!(
            c.progress.iter().map(|p| p.episode).collect::<Vec<_>>(),
            [2]
        );
        assert_eq!(l.changes_since(0).unwrap().entries.len(), 2);
    }

    #[test]
    fn the_newer_edit_of_a_title_wins_from_either_side() {
        let l = lib();
        l.apply(&changes(vec![entry_at(1, "watching", 100)], vec![]))
            .unwrap();
        let newer = LibraryEntry {
            chosen_dub_key: Some("anidub".into()),
            ..entry_at(1, "completed", 200)
        };
        assert_eq!(l.apply(&changes(vec![newer], vec![])).unwrap(), (1, 0));
        assert_eq!(
            l.apply(&changes(vec![entry_at(1, "planned", 150)], vec![]))
                .unwrap(),
            (0, 0)
        );
        let e = l.get_entry(1).unwrap().unwrap();
        assert_eq!(
            (e.status.as_str(), e.chosen_dub_key.as_deref()),
            ("completed", Some("anidub"))
        );
        assert_eq!(e.added_at, 100, "a title keeps the day it was first added");
        assert_eq!(
            l.apply(&changes(vec![entry_at(7, "planned", 50)], vec![]))
                .unwrap(),
            (1, 0)
        );
    }

    #[test]
    fn a_watched_episode_stays_watched_and_the_place_follows_the_newer_edit() {
        let l = lib();
        l.apply(&changes(vec![], vec![progress_at(3, 1400.0, true, 100)]))
            .unwrap();
        // Rewatching on another device: an older "watched" never comes back as "not watched".
        l.apply(&changes(vec![], vec![progress_at(3, 300.0, false, 200)]))
            .unwrap();
        let row = l.progress(1).unwrap().remove(0);
        assert_eq!(
            (row.position_sec, row.watched, row.updated_at),
            (300.0, true, 200)
        );
        // An older place does not move it back.
        assert_eq!(
            l.apply(&changes(vec![], vec![progress_at(3, 50.0, false, 150)]))
                .unwrap(),
            (0, 0)
        );
        assert_eq!(l.progress(1).unwrap()[0].position_sec, 300.0);
    }

    #[test]
    fn a_backup_restores_the_library_into_an_empty_one() {
        let from = lib();
        from.set_status(1, "completed").unwrap();
        from.set_dub(2, "anidub").unwrap();
        from.set_status(2, "watching").unwrap();
        from.save_progress(&p(3, 1440.0, true), None, None).unwrap();
        let backup = from.export().unwrap();
        let json = serde_json::to_string(&backup).unwrap();

        let to = lib();
        let restored = to.restore(&serde_json::from_str(&json).unwrap()).unwrap();
        assert_eq!(restored, (2, 1));
        assert_eq!(
            to.get_entry(2).unwrap().unwrap().chosen_dub_key.as_deref(),
            Some("anidub")
        );
        assert_eq!(to.progress(1).unwrap().len(), 1);
    }

    #[test]
    fn a_backup_does_not_overwrite_what_the_library_has() {
        let from = lib();
        from.set_status(1, "completed").unwrap();
        from.set_status(5, "planned").unwrap();
        let to = lib();
        to.set_status(1, "dropped").unwrap();
        let restored = to.restore(&from.export().unwrap()).unwrap();
        assert_eq!(restored, (1, 0));
        assert_eq!(to.get_entry(1).unwrap().unwrap().status, "dropped");
        assert_eq!(to.get_entry(5).unwrap().unwrap().status, "planned");
    }

    #[test]
    fn an_imported_title_brings_its_status_and_watched_episodes() {
        let l = lib();
        assert!(l.import_rate(7, "watching", 3, 500).unwrap());
        assert_eq!(l.get_entry(7).unwrap().unwrap().status, "watching");
        let progress = l.progress(7).unwrap();
        assert_eq!(
            progress.iter().map(|p| p.episode).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert!(progress.iter().all(|p| p.watched && p.updated_at == 500));
    }

    #[test]
    fn titles_already_in_the_library_are_not_touched_by_an_import() {
        let l = lib();
        l.set_status(7, "dropped").unwrap();
        assert!(!l.import_rate(7, "completed", 12, 500).unwrap());
        assert_eq!(l.get_entry(7).unwrap().unwrap().status, "dropped");
        assert!(l.progress(7).unwrap().is_empty());
    }

    #[test]
    fn an_import_keeps_progress_already_made_on_a_hidden_title() {
        let l = lib();
        l.set_dub(7, "anidub").unwrap();
        let mut started = p(2, 300.0, false);
        started.shikimori_id = 7;
        l.save_progress(&started, None, None).unwrap();
        l.remove(7).unwrap();
        assert!(l.import_rate(7, "watching", 3, 500).unwrap());
        let two = l
            .progress(7)
            .unwrap()
            .into_iter()
            .find(|x| x.episode == 2)
            .unwrap();
        assert_eq!(two.position_sec, 300.0, "the progress made here stays");
    }

    #[test]
    fn the_last_watch_is_the_latest_progress_update() {
        let mut late = p(1, 600.0, false);
        late.updated_at = 900;
        assert_eq!(last_watched(&[p(3, 100.0, false), late]), Some(900));
        assert_eq!(last_watched(&[]), None);
    }

    #[test]
    fn watched_rules() {
        assert!(!is_watched(100.0, 1440.0, None));
        assert!(is_watched(1300.0, 1440.0, None));
        assert!(is_watched(1250.0, 1440.0, Some(1240.0)));
        assert!(!is_watched(0.0, 0.0, None));
    }

    #[test]
    fn episodes_out_since_the_last_one_opened() {
        assert_eq!(
            fresh_episodes(&[p(1, 1440.0, true), p(2, 1440.0, true)], 5),
            3
        );
        assert_eq!(
            fresh_episodes(&[p(1, 1440.0, true), p(2, 300.0, false)], 2),
            0,
            "the last aired episode is already open"
        );
        assert_eq!(fresh_episodes(&[], 5), 0, "nothing watched, nothing is new");
    }

    #[test]
    fn a_chosen_episode_resumes_where_it_was_left() {
        assert_eq!(resume_position(None), 0.0);
        assert_eq!(resume_position(Some(&p(3, 600.0, false))), 595.0);
        assert_eq!(
            resume_position(Some(&p(3, 600.0, true))),
            0.0,
            "watched starts over"
        );
        assert_eq!(
            resume_position(Some(&p(3, 8.0, false))),
            0.0,
            "barely started"
        );
    }

    #[test]
    fn continue_picks_unfinished_or_next() {
        assert_eq!(
            continue_target(&[p(1, 1440.0, true), p(2, 600.0, false)], 12),
            Some((2, 595.0))
        );
        assert_eq!(
            continue_target(&[p(1, 1440.0, true), p(2, 1440.0, true)], 12),
            Some((3, 0.0))
        );
        assert_eq!(continue_target(&[p(12, 1440.0, true)], 12), None);
        assert_eq!(continue_target(&[], 12), Some((1, 0.0)));
        assert_eq!(continue_target(&[], 0), None);
        assert_eq!(continue_target(&[p(1, 3.0, false)], 12), Some((1, 0.0)));
    }

    #[test]
    fn first_progress_starts_watching_and_marks_watched() {
        let l = lib();
        let mut e = p(1, 1300.0, false);
        e.dub_key = Some("anidub".into());
        l.save_progress(&e, None, None).unwrap();
        let saved = l.progress(1).unwrap();
        assert!(saved[0].watched);
        assert_eq!(saved[0].dub_key.as_deref(), Some("anidub"));
        assert_eq!(l.get_entry(1).unwrap().unwrap().status, "watching");
    }

    #[test]
    fn watched_flag_is_sticky_on_rewatch() {
        let l = lib();
        l.save_progress(&p(1, 1400.0, false), None, None).unwrap();
        l.save_progress(&p(1, 10.0, false), None, None).unwrap();
        let saved = l.progress(1).unwrap();
        assert!(saved[0].watched);
        assert_eq!(saved[0].position_sec, 10.0);
    }

    #[test]
    fn last_episode_completes_released_title() {
        let l = lib();
        l.save_progress(&p(12, 1400.0, false), None, Some(12))
            .unwrap();
        assert_eq!(l.get_entry(1).unwrap().unwrap().status, "completed");
    }

    #[test]
    fn remembered_dub_does_not_add_title_to_list() {
        let l = lib();
        l.set_dub(5, "anidub").unwrap();
        assert!(l.list(None).unwrap().is_empty());
        assert_eq!(
            l.get_entry(5).unwrap().unwrap().chosen_dub_key.as_deref(),
            Some("anidub")
        );
        l.set_status(5, "planned").unwrap();
        assert_eq!(l.list(Some("planned")).unwrap().len(), 1);
        assert_eq!(
            l.get_entry(5).unwrap().unwrap().chosen_dub_key.as_deref(),
            Some("anidub")
        );
    }

    #[test]
    fn continue_list_has_latest_row_per_watching_title() {
        let l = lib();
        let mut a = p(1, 100.0, false);
        a.updated_at = 10;
        let mut b = p(2, 200.0, false);
        b.updated_at = 20;
        let mut c = p(1, 50.0, false);
        c.shikimori_id = 2;
        c.updated_at = 30;
        for x in [&a, &b, &c] {
            l.save_progress(x, None, None).unwrap();
        }
        let rows = l.continue_list().unwrap();
        let got: Vec<_> = rows.iter().map(|r| (r.shikimori_id, r.episode)).collect();
        assert_eq!(got, vec![(2, 1), (1, 2)]);
    }

    #[test]
    fn continue_list_has_one_row_per_title_when_saves_share_a_second() {
        let l = lib();
        let (mut a, mut b) = (p(1, 1440.0, true), p(2, 30.0, false));
        a.updated_at = 50;
        b.updated_at = 50;
        l.save_progress(&a, None, None).unwrap();
        l.save_progress(&b, None, None).unwrap();
        let got: Vec<_> = l
            .continue_list()
            .unwrap()
            .iter()
            .map(|r| (r.shikimori_id, r.episode))
            .collect();
        assert_eq!(got, vec![(1, 2)]);
    }

    #[test]
    fn a_save_without_duration_keeps_the_known_one() {
        let l = lib();
        l.save_progress(&p(1, 600.0, false), None, None).unwrap();
        let mut early = p(1, 620.0, false);
        early.duration_sec = 0.0;
        l.save_progress(&early, None, None).unwrap();
        let row = &l.progress(1).unwrap()[0];
        assert_eq!((row.position_sec, row.duration_sec), (620.0, 1440.0));
    }

    #[test]
    fn rejects_unknown_status() {
        assert_eq!(
            lib().set_status(1, "лучшее").unwrap_err().kind,
            "BadRequest"
        );
    }
}
