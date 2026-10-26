//! Watches the titles the user follows and tells them when a new episode becomes available.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use crate::catalog::sequels::new_seasons;
use crate::catalog::Catalog;
use crate::db::{get_setting, set_setting, Db};
use crate::error::AppError;
use crate::library::Library;
use crate::sources::Resolver;

/// Whether to tell the user about new episodes; on unless they turn it off.
pub const NOTIFY_SETTING: &str = "notify.enabled";
/// The latest episode already announced for each followed title.
const SEEN_SETTING: &str = "notify.seen";
/// Sequels of finished titles already announced.
const SEQUELS_SEEN_SETTING: &str = "notify.sequels_seen";
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(60);
const CHECK_EVERY: Duration = Duration::from_secs(30 * 60);

/// A title the user follows and the latest episode its sources have.
pub struct Followed {
    pub id: i64,
    pub title: String,
    pub latest: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub id: i64,
    pub title: String,
    pub from: i64,
    pub to: i64,
}

impl Alert {
    /// The notification's title and text.
    pub fn text(&self) -> (String, String) {
        let body = if self.from == self.to {
            format!("Вышла {} серия", self.to)
        } else {
            format!("Вышли серии {}-{}", self.from, self.to)
        };
        (self.title.clone(), body)
    }
}

/// Alerts for episodes newer than the ones already seen, and what is seen after them. A title
/// seen for the first time is only remembered, so the first check after install stays quiet.
pub fn new_episodes(
    followed: &[Followed],
    seen: &HashMap<i64, i64>,
) -> (Vec<Alert>, HashMap<i64, i64>) {
    let mut alerts = Vec::new();
    let mut next = HashMap::new();
    for f in followed {
        let last = match seen.get(&f.id) {
            Some(&last) => last,
            None => {
                next.insert(f.id, f.latest);
                continue;
            }
        };
        if f.latest > last {
            alerts.push(Alert {
                id: f.id,
                title: f.title.clone(),
                from: last + 1,
                to: f.latest,
            });
        }
        next.insert(f.id, f.latest.max(last));
    }
    (alerts, next)
}

/// Sequels out now that were not announced yet; nothing on the very first check (`seen` is
/// None), which only remembers what is already out.
pub fn sequels_to_announce(current: &[i64], seen: Option<&HashSet<i64>>) -> Vec<i64> {
    match seen {
        None => Vec::new(),
        Some(seen) => current
            .iter()
            .copied()
            .filter(|id| !seen.contains(id))
            .collect(),
    }
}

/// Shows a notification with a title and a text.
pub type Notify = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// Services the watcher needs.
pub struct Watcher {
    pub notify: Notify,
    pub db: Db,
    pub library: Arc<Library>,
    pub catalog: Arc<Catalog>,
    pub resolver: Arc<Resolver>,
}

impl Watcher {
    /// Checks in the background on `rt`, for as long as the app runs.
    pub fn spawn(self, rt: &tokio::runtime::Handle) {
        rt.spawn(async move {
            tokio::time::sleep(FIRST_CHECK_AFTER).await;
            loop {
                if self.enabled() {
                    if let Err(e) = self.check().await {
                        tracing::warn!("new episode check failed: {e}");
                    }
                }
                tokio::time::sleep(CHECK_EVERY).await;
            }
        });
    }

    fn enabled(&self) -> bool {
        let conn = self.db.lock().unwrap();
        get_setting(&conn, NOTIFY_SETTING)
            .ok()
            .flatten()
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    }

    async fn followed(&self) -> Result<Vec<Followed>, AppError> {
        let mut out = Vec::new();
        for e in self
            .library
            .list(Some("watching"))?
            .into_iter()
            .filter(|e| e.notify)
        {
            let Ok(anime) = self.catalog.get(e.shikimori_id).await else {
                continue;
            };
            if !anime.is_ongoing() {
                continue;
            }
            let Ok(sources) = self.resolver.title_sources(e.shikimori_id).await else {
                continue;
            };
            let latest = sources
                .dubs
                .iter()
                .flat_map(|d| d.episodes.iter().copied())
                .max()
                .unwrap_or(0);
            out.push(Followed {
                id: e.shikimori_id,
                title: anime.russian.unwrap_or(anime.name),
                latest,
            });
        }
        Ok(out)
    }

    async fn check(&self) -> Result<(), AppError> {
        let followed = self.followed().await?;
        let seen: HashMap<i64, i64> = {
            let conn = self.db.lock().unwrap();
            get_setting(&conn, SEEN_SETTING)
                .ok()
                .flatten()
                .and_then(|v| serde_json::from_value::<HashMap<String, i64>>(v).ok())
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(k, v)| Some((k.parse().ok()?, v)))
                .collect()
        };
        let (alerts, next) = new_episodes(&followed, &seen);
        {
            let conn = self.db.lock().unwrap();
            let map: HashMap<String, i64> =
                next.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
            set_setting(&conn, SEEN_SETTING, &serde_json::json!(map))?;
        }
        for alert in alerts {
            let (title, body) = alert.text();
            self.notify(title, body);
        }
        self.check_sequels().await
    }

    fn notify(&self, title: String, body: String) {
        (self.notify)(&title, &body);
    }

    /// Tells about a new season of a finished title once it starts airing.
    async fn check_sequels(&self) -> Result<(), AppError> {
        let seasons = new_seasons(&self.catalog, &self.library).await?;
        let current: Vec<i64> = seasons.iter().map(|n| n.anime.id).collect();
        let seen: Option<HashSet<i64>> = {
            let conn = self.db.lock().unwrap();
            get_setting(&conn, SEQUELS_SEEN_SETTING)
                .ok()
                .flatten()
                .and_then(|v| serde_json::from_value(v).ok())
        };
        // Known from an empty library is known of nothing: the list from Shikimori lands after.
        let known = seen.as_ref().filter(|s| !s.is_empty());
        for id in sequels_to_announce(&current, known) {
            // Only a season airing now is news; one that ended long ago is in "new seasons".
            if let Some(n) = seasons
                .iter()
                .find(|n| n.anime.id == id && n.anime.status.as_deref() == Some("ongoing"))
            {
                let after = n
                    .after
                    .russian
                    .clone()
                    .unwrap_or_else(|| n.after.name.clone());
                let sequel = n
                    .anime
                    .russian
                    .clone()
                    .unwrap_or_else(|| n.anime.name.clone());
                self.notify(
                    format!("Вышло продолжение «{after}»"),
                    format!("«{sequel}» уже можно смотреть"),
                );
            }
        }
        let mut all = seen.unwrap_or_default();
        all.extend(current);
        let conn = self.db.lock().unwrap();
        set_setting(&conn, SEQUELS_SEEN_SETTING, &serde_json::json!(all))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(pairs: &[(i64, i64)]) -> HashMap<i64, i64> {
        pairs.iter().copied().collect()
    }

    #[test]
    fn a_title_seen_for_the_first_time_is_only_remembered() {
        let (alerts, next) = new_episodes(
            &[Followed {
                id: 1,
                title: "Фрирен".into(),
                latest: 4,
            }],
            &seen(&[]),
        );
        assert!(alerts.is_empty());
        assert_eq!(next, seen(&[(1, 4)]));
    }

    #[test]
    fn a_newer_episode_is_announced_once() {
        let followed = [Followed {
            id: 1,
            title: "Фрирен".into(),
            latest: 5,
        }];
        let (alerts, next) = new_episodes(&followed, &seen(&[(1, 4)]));
        assert_eq!(
            alerts,
            [Alert {
                id: 1,
                title: "Фрирен".into(),
                from: 5,
                to: 5
            }]
        );
        let (again, _) = new_episodes(&followed, &next);
        assert!(again.is_empty());
    }

    #[test]
    fn several_new_episodes_make_one_alert() {
        let followed = [Followed {
            id: 1,
            title: "Фрирен".into(),
            latest: 7,
        }];
        let (alerts, _) = new_episodes(&followed, &seen(&[(1, 4)]));
        assert_eq!(
            alerts,
            [Alert {
                id: 1,
                title: "Фрирен".into(),
                from: 5,
                to: 7
            }]
        );
    }

    #[test]
    fn titles_no_longer_followed_are_forgotten_and_fewer_episodes_change_nothing() {
        let followed = [Followed {
            id: 2,
            title: "Наруто".into(),
            latest: 3,
        }];
        let (alerts, next) = new_episodes(&followed, &seen(&[(1, 4), (2, 5)]));
        assert!(alerts.is_empty(), "a source dropping an episode is no news");
        assert_eq!(next, seen(&[(2, 5)]));
    }

    #[test]
    fn sequels_are_announced_once_and_not_on_the_first_check() {
        assert!(sequels_to_announce(&[2, 3], None).is_empty());
        let seen = HashSet::from([2]);
        assert_eq!(sequels_to_announce(&[2, 3], Some(&seen)), vec![3]);
    }

    #[test]
    fn alerts_read_naturally() {
        let one = Alert {
            id: 1,
            title: "Фрирен".into(),
            from: 5,
            to: 5,
        };
        assert_eq!(
            one.text(),
            ("Фрирен".to_string(), "Вышла 5 серия".to_string())
        );
        let many = Alert {
            id: 1,
            title: "Фрирен".into(),
            from: 5,
            to: 7,
        };
        assert_eq!(many.text().1, "Вышли серии 5-7");
    }
}
