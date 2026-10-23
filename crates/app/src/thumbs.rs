//! Pictures from the web the core does not keep itself (video thumbnails, frames trace.moe
//! found), downloaded once into the cache.

use std::collections::HashMap;
use std::path::PathBuf;

use gpui::{App, Global};

use crate::core::{Core, cache_dir};

#[derive(Default)]
struct Fetched(HashMap<String, Option<PathBuf>>);

impl Global for Fetched {}

/// YouTube ids are 11 letters, digits, '-' and '_'; anything else is not one (and must not
/// become a file name).
pub fn valid_id(id: &str) -> bool {
    id.len() == 11
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The thumbnail of video `id`, or `None` until it is downloaded; the windows redraw then.
pub fn youtube_thumb(id: &str, cx: &mut App) -> Option<PathBuf> {
    if !valid_id(id) {
        return None;
    }
    // Known ones first: the disk is asked once per video, not on every frame.
    if let Some(known) = cx.default_global::<Fetched>().0.get(id) {
        return known.clone();
    }
    let path = cache_dir().join("thumbs").join(format!("{id}.jpg"));
    if path.exists() {
        cx.global_mut::<Fetched>()
            .0
            .insert(id.into(), Some(path.clone()));
        return Some(path);
    }
    cx.global_mut::<Fetched>().0.insert(id.into(), None);
    let url = format!("https://img.youtube.com/vi/{id}/mqdefault.jpg");
    let core = Core::get(cx);
    let (http, target) = (core.http.clone(), path.clone());
    let job = core.rt.spawn(async move {
        let bytes = http
            .inner()
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        std::fs::create_dir_all(cache_dir().join("thumbs"))?;
        std::fs::write(&target, &bytes)?;
        anyhow::Ok(target)
    });
    let id = id.to_string();
    cx.spawn(async move |cx| {
        let saved = match job.await {
            Ok(Ok(path)) => Some(path),
            Ok(Err(e)) => {
                tracing::info!("thumbnail of {id}: {e}");
                None
            }
            Err(_) => None,
        };
        cx.update(|cx| {
            cx.global_mut::<Fetched>().0.insert(id, saved);
            cx.refresh_windows();
        });
    })
    .detach();
    None
}

/// A picture at `url`, or `None` until it is downloaded; the windows redraw then. Cached by a
/// hash of the address.
pub fn remote_image(url: &str, cx: &mut App) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hash);
    let key = format!("{:016x}", hash.finish());
    if let Some(known) = cx.default_global::<Fetched>().0.get(&key) {
        return known.clone();
    }
    let path = cache_dir().join("remote").join(&key);
    if path.exists() {
        cx.global_mut::<Fetched>().0.insert(key, Some(path.clone()));
        return Some(path);
    }
    cx.global_mut::<Fetched>().0.insert(key.clone(), None);
    let core = Core::get(cx);
    let (http, url, target) = (core.http.clone(), url.to_string(), path);
    let job = core.rt.spawn(async move {
        let bytes = http
            .inner()
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        std::fs::create_dir_all(cache_dir().join("remote"))?;
        std::fs::write(&target, &bytes)?;
        anyhow::Ok(target)
    });
    cx.spawn(async move |cx| {
        let saved = job.await.ok().and_then(Result::ok);
        cx.update(|cx| {
            cx.global_mut::<Fetched>().0.insert(key, saved);
            cx.refresh_windows();
        });
    })
    .detach();
    None
}
