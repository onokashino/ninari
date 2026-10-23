//! Posters not downloaded yet are fetched the first time a card shows them, once per title.

use std::collections::HashMap;
use std::path::PathBuf;

use gpui::{App, Global};
use ninari_core::catalog::posters::Palette;
use ninari_core::feeds::AnimeCard;

use crate::core::Core;

enum Fetch {
    Running,
    /// The poster, or `None` when the title has none.
    Done(Option<(PathBuf, Palette)>),
}

/// Downloads asked for, by title.
#[derive(Default)]
struct Fetched(HashMap<i64, Fetch>);

impl Global for Fetched {}

/// The poster file of `card` and its palette, or `None` until it is downloaded; the windows
/// redraw when it lands.
pub fn poster(card: &AnimeCard, cx: &mut App) -> Option<(PathBuf, Palette)> {
    if let Some(path) = &card.poster_path {
        return Some((path.into(), card.palette.clone().unwrap_or_default()));
    }
    let id = card.anime.id;
    match cx.default_global::<Fetched>().0.get(&id) {
        Some(Fetch::Done(found)) => return found.clone(),
        Some(Fetch::Running) => return None,
        None => {}
    }
    cx.global_mut::<Fetched>().0.insert(id, Fetch::Running);
    let anime = card.anime.clone();
    let download = Core::get(cx).run(move |c| async move { c.posters.ensure(&anime).await });
    cx.spawn(async move |cx| {
        let found = download.await.unwrap_or_else(|e| {
            tracing::info!("poster of {id}: {e:#}");
            None
        });
        cx.update(|cx| {
            cx.global_mut::<Fetched>().0.insert(id, Fetch::Done(found));
            cx.refresh_windows();
        });
    })
    .detach();
    None
}

/// True once it is known that the title has no poster at all.
pub fn missing(card: &AnimeCard, cx: &App) -> bool {
    card.poster_path.is_none()
        && cx
            .try_global::<Fetched>()
            .is_some_and(|f| matches!(f.0.get(&card.anime.id), Some(Fetch::Done(None))))
}
