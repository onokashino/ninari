//! A random title, from the rail or the catalog, opened at once; again for another one. What it
//! may be (genres, years, a studio) is set once and kept.

use std::collections::HashSet;
use std::sync::Arc;

use ninari_core::catalog::browse::BrowseFilter;
use serde::{Deserialize, Serialize};

use crate::core::Core;

pub const SETTING: &str = "random.filter";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RandomFilter {
    /// Shikimori genre ids, all of which the title has.
    #[serde(default)]
    pub genres: Vec<i64>,
    /// Years as Shikimori takes them: "2010_2019".
    #[serde(default)]
    pub years: Option<String>,
    #[serde(default)]
    pub studio: Option<i64>,
}

impl RandomFilter {
    pub fn count(&self) -> usize {
        self.genres.len() + usize::from(self.years.is_some()) + usize::from(self.studio.is_some())
    }

    fn browse(&self) -> BrowseFilter {
        BrowseFilter {
            genres: self.genres.clone(),
            season: self.years.clone(),
            studio: self.studio,
            // Something worth watching: series and films that are out, rated 6 or more, with
            // episodes over ten minutes. The whole catalog at random is mostly obscure shorts.
            kinds: ["tv", "movie", "ova", "ona"].map(String::from).to_vec(),
            statuses: ["released", "ongoing"].map(String::from).to_vec(),
            durations: ["D", "F"].map(String::from).to_vec(),
            score: Some(6),
            order: Some("random".into()),
            ..Default::default()
        }
    }
}

pub const YEARS: [(&str, &str); 5] = [
    ("2020_2029", "2020-е"),
    ("2010_2019", "2010-е"),
    ("2000_2009", "2000-е"),
    ("1990_1999", "1990-е"),
    ("1960_1989", "Раньше"),
];

/// Well known studios with their Shikimori ids (which do not change); few enough to fit the window.
pub const STUDIOS: [(i64, &str); 20] = [
    (11, "Madhouse"),
    (4, "Bones"),
    (569, "MAPPA"),
    (2, "Kyoto Animation"),
    (43, "ufotable"),
    (858, "Wit Studio"),
    (56, "A-1 Pictures"),
    (1835, "CloverWorks"),
    (10, "Production I.G"),
    (14, "Sunrise"),
    (18, "Toei Animation"),
    (44, "Shaft"),
    (803, "Trigger"),
    (7, "J.C.Staff"),
    (37, "Studio Deen"),
    (132, "P.A. Works"),
    (95, "Doga Kobo"),
    (21, "Studio Ghibli"),
    (314, "White Fox"),
    (1591, "Science SARU"),
];

/// A title by `filter` not among `seen`, or `None` when nothing matches.
pub fn roll(
    core: &Arc<Core>,
    filter: &RandomFilter,
    seen: HashSet<i64>,
) -> impl Future<Output = anyhow::Result<Option<i64>>> + use<> {
    let filter = filter.browse();
    core.run(move |c| async move {
        let mut found = None;
        // Shikimori's random may repeat itself: a few tries for one not shown yet.
        for _ in 0..5 {
            found = c
                .catalog
                .browse(&filter, 1)
                .await?
                .into_iter()
                .next()
                .map(|a| a.id);
            if !found.is_some_and(|id| seen.contains(&id)) {
                break;
            }
        }
        Ok(found)
    })
}

/// The random title as Root keeps it.
#[derive(Default)]
pub struct RandomState {
    pub filter: RandomFilter,
    /// Titles shown already this run: the next roll avoids them.
    pub seen: HashSet<i64>,
    /// The title the last roll opened.
    pub current: Option<i64>,
    pub rolling: bool,
    /// The settings are open.
    pub settings: bool,
    /// The genres to offer, once fetched.
    pub genres: Option<Vec<ninari_core::catalog::browse::Genre>>,
    /// Why a roll found nothing.
    pub note: Option<String>,
}
