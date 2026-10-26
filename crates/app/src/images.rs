//! Pictures kept decoded in memory up to a budget. GPUI's own cache keeps every picture ever
//! shown until the app quits: a poster is about a megabyte decoded, so an evening of browsing
//! grew to gigabytes.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    App, AppContext, Entity, ImageCache, ImageCacheError, ImageCacheItem, RenderImage, Resource,
    WeakEntity, Window, hash,
};

/// Decoded pictures kept beyond the ones on screen.
const BUDGET: usize = 192 << 20;

struct Entry {
    item: ImageCacheItem,
    /// Decoded size, 0 until it has loaded.
    bytes: usize,
    /// The frame that last drew it.
    seen: u64,
}

/// The window's picture cache, for elements that paint pictures themselves (card_element).
pub struct Current(pub Entity<Images>);

impl gpui::Global for Current {}

pub struct Images {
    entries: HashMap<u64, Entry>,
    bytes: usize,
    budget: usize,
    frame: u64,
    /// A sweep waits for the end of the frame.
    sweeping: bool,
    this: WeakEntity<Self>,
}

impl Images {
    pub fn new(cx: &mut App) -> Entity<Self> {
        Self::with_budget(BUDGET, cx)
    }

    fn with_budget(budget: usize, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            entries: HashMap::new(),
            bytes: 0,
            budget,
            frame: 0,
            sweeping: false,
            this: cx.weak_entity(),
        })
    }

    /// Runs after a frame: what it drew stays, the rest goes oldest first while over budget.
    fn sweep(&mut self, window: &mut Window, cx: &mut App) {
        self.sweeping = false;
        let seen = self
            .entries
            .iter()
            .map(|(&key, e)| (key, e.seen, e.bytes))
            .collect::<Vec<_>>();
        for key in victims(seen, self.bytes, self.budget, self.frame) {
            if let Some(e) = self.entries.remove(&key) {
                self.bytes -= e.bytes;
                if let Some(Ok(image)) = e.item.get() {
                    cx.drop_image(image, Some(window));
                }
            }
        }
        self.frame += 1;
    }
}

impl ImageCache for Images {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let frame = self.frame;
        let entry = self.entries.entry(hash(resource)).or_insert_with(|| Entry {
            item: ImageCacheItem::new(resource, cx),
            bytes: 0,
            seen: frame,
        });
        entry.seen = frame;
        let result = entry.item.use_image(window);
        if entry.bytes == 0
            && let Some(Ok(image)) = &result
        {
            entry.bytes = (0..image.frame_count())
                .map(|i| {
                    let size = image.size(i);
                    size.width.0 as usize * size.height.0 as usize * 4
                })
                .sum::<usize>()
                .max(1);
            self.bytes += entry.bytes;
        }
        if !self.sweeping {
            self.sweeping = true;
            let this = self.this.clone();
            window.on_next_frame(move |window, cx| {
                this.update(cx, |images, cx| images.sweep(window, cx)).ok();
            });
        }
        result
    }
}

/// Which pictures to drop: `(key, last frame drawn, bytes)` oldest first until `total` fits
/// `budget`, never one drawn on `frame`.
fn victims(
    mut seen: Vec<(u64, u64, usize)>,
    mut total: usize,
    budget: usize,
    frame: u64,
) -> Vec<u64> {
    if total <= budget {
        return Vec::new();
    }
    seen.retain(|&(_, at, bytes)| at < frame && bytes > 0);
    seen.sort_unstable_by_key(|&(_, at, _)| at);
    let mut out = Vec::new();
    for (key, _, bytes) in seen {
        if total <= budget {
            break;
        }
        total -= bytes;
        out.push(key);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gpui::{
        Context, Entity, IntoElement, ParentElement, Render, Resource, Styled, TestAppContext,
        Window, div, hash, image_cache, img, px,
    };

    use super::{Images, victims};

    struct Wall {
        files: Vec<PathBuf>,
        images: Entity<Images>,
    }

    impl Render for Wall {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let pictures = self.files.iter().map(|f| img(f.clone()).size(px(32.)));
            image_cache(self.images.clone())
                .size_full()
                .child(div().flex().flex_wrap().children(pictures))
        }
    }

    #[gpui::test]
    fn scrolling_through_many_pictures_stays_in_budget(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!("ninari-images-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let files: Vec<PathBuf> = (0..300)
            .map(|i| {
                let path = dir.join(format!("{i}.png"));
                image::RgbaImage::from_pixel(64, 64, image::Rgba([i as u8, 0, 0, 255]))
                    .save(&path)
                    .unwrap();
                path
            })
            .collect();
        // 16 KiB a picture: the budget holds 64 of them, a screen shows 20.
        let budget = 1 << 20;
        let images = cx.update(|cx| Images::with_budget(budget, cx));
        let (wall, cx) = cx.add_window_view(|_, _| Wall {
            files: Vec::new(),
            images: images.clone(),
        });
        for screen in files.chunks(20) {
            wall.update(cx, |w, _| w.files = screen.to_vec());
            // The second draw finds the pictures decoded.
            for _ in 0..2 {
                cx.update(|window, cx| {
                    window.refresh();
                    window.draw(cx).clear(cx);
                });
                cx.run_until_parked();
            }
            cx.update(|window, cx| window.simulate_next_frame(cx));
        }
        let last = files[280..].to_vec();
        cx.update(|_, cx| {
            let images = images.read(cx);
            assert!(images.bytes <= budget, "{} over {budget}", images.bytes);
            assert!(images.entries.len() < 100, "{} kept", images.entries.len());
            for f in &last {
                let entry = &images.entries[&hash(&Resource::Path(f.as_path().into()))];
                assert!(entry.bytes > 0, "{f:?} on screen but not decoded");
            }
        });
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn drops_the_oldest_until_it_fits_but_never_what_is_on_screen() {
        // key, frame, bytes; frame 5 is the one just drawn.
        let seen = vec![(1, 3, 10), (2, 1, 10), (3, 5, 50), (4, 2, 10), (5, 0, 0)];
        assert_eq!(victims(seen.clone(), 80, 100, 5), Vec::<u64>::new());
        assert_eq!(victims(seen.clone(), 80, 65, 5), vec![2, 4]);
        // Even all the old ones gone does not touch the picture on screen.
        assert_eq!(victims(seen, 80, 10, 5), vec![2, 4, 1]);
    }
}
