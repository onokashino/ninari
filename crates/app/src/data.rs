//! Shared pieces of what the screens show.

use std::future::Future;

use gpui::Context;
use ninari_core::catalog::Anime;

/// Something a screen waits for.
pub enum Load<T> {
    Pending,
    Ready(T),
    Failed(String),
}

impl<T> From<anyhow::Result<T>> for Load<T> {
    fn from(r: anyhow::Result<T>) -> Self {
        match r {
            Ok(v) => Load::Ready(v),
            Err(e) => Load::Failed(format!("{e:#}")),
        }
    }
}

/// The Russian name when there is one.
pub fn title(anime: &Anime) -> String {
    anime
        .russian
        .clone()
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| anime.name.clone())
}

/// Runs `job` and hands what it brings to the view with `put`, then redraws it.
pub fn fetch<V: 'static, T: 'static>(
    cx: &mut Context<V>,
    job: impl Future<Output = anyhow::Result<T>> + 'static,
    put: impl FnOnce(&mut V, Load<T>) + 'static,
) {
    cx.spawn(async move |view, cx| {
        let got = job.await;
        if let Err(e) = &got {
            tracing::warn!("{} failed: {e:#}", std::any::type_name::<T>());
        }
        view.update(cx, |view, cx| {
            put(view, got.into());
            cx.notify();
        })
        .ok();
    })
    .detach();
}
