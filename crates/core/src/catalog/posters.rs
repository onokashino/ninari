use std::path::PathBuf;
use std::sync::Arc;

use image::DynamicImage;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::anilist::AniList;
use super::Anime;
use crate::db::Db;
use crate::error::AppError;
use crate::net::HttpClient;

/// Wider than this a poster is only ever drawn smaller: a card is up to ~220 points wide, twice
/// that on a HiDPI screen. Some covers come 1900 wide, 20 MB decoded, and their upload to the GPU
/// was a visible hitch while scrolling.
pub const POSTER_MAX_W: u32 = 440;

/// `bytes` of a picture made at most `max` wide, in its own format; `None` when it already is or
/// cannot be read.
pub fn shrink(bytes: &[u8], max: u32) -> Option<Vec<u8>> {
    let format = image::guess_format(bytes).ok()?;
    let img = image::load_from_memory_with_format(bytes, format).ok()?;
    if img.width() <= max {
        return None;
    }
    let height = (u64::from(img.height()) * u64::from(max) / u64::from(img.width())).max(1) as u32;
    let small = img.resize_exact(max, height, image::imageops::FilterType::CatmullRom);
    let mut out = std::io::Cursor::new(Vec::new());
    match format {
        image::ImageFormat::Jpeg => {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
                .encode_image(&small.to_rgb8())
                .ok()?;
        }
        _ => small.write_to(&mut out, format).ok()?,
    }
    Some(out.into_inner())
}

pub const DEFAULT_ACCENT: &str = "#b9a3ff";
pub const DEFAULT_GLOW: &str = "#3a2f6b";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Palette {
    pub accent: String,
    pub glow: String,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            accent: DEFAULT_ACCENT.into(),
            glow: DEFAULT_GLOW.into(),
        }
    }
}

fn rgb_to_hsl(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f64::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        ((g - b) / d + if g < b { 6.0 } else { 0.0 }) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, l)
}

fn hsl_to_hex(h: f64, s: f64, l: f64) -> String {
    let hue = |p: f64, q: f64, mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let to8 = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        to8(hue(p, q, h + 1.0 / 3.0)),
        to8(hue(p, q, h)),
        to8(hue(p, q, h - 1.0 / 3.0))
    )
}

/// Picks the dominant saturated hue of a poster and derives a readable accent (light)
/// and a background glow (dark) from it. Gray or very dark posters get the default palette.
pub fn pick_palette(img: &DynamicImage) -> Palette {
    let small = img.thumbnail(32, 32).to_rgb8();
    // weight, r, g, b sums per hue bin
    let mut bins = [(0.0f64, 0.0f64, 0.0f64, 0.0f64); 12];
    for px in small.pixels() {
        let (r, g, b) = (
            px[0] as f64 / 255.0,
            px[1] as f64 / 255.0,
            px[2] as f64 / 255.0,
        );
        let (h, s, l) = rgb_to_hsl(r, g, b);
        if !(0.2..=0.92).contains(&l) || s < 0.25 {
            continue;
        }
        let bin = ((h * 12.0) as usize).min(11);
        bins[bin].0 += s;
        bins[bin].1 += r * s;
        bins[bin].2 += g * s;
        bins[bin].3 += b * s;
    }
    let best = bins
        .iter()
        .copied()
        .fold((0.0, 0.0, 0.0, 0.0), |a, b| if b.0 > a.0 { b } else { a });
    if best.0 <= 0.0 {
        return Palette::default();
    }
    let (h, s, _) = rgb_to_hsl(best.1 / best.0, best.2 / best.0, best.3 / best.0);
    let s = s.clamp(0.45, 0.9);
    Palette {
        accent: hsl_to_hex(h, s, 0.68),
        glow: hsl_to_hex(h, s * 0.8, 0.26),
    }
}

pub struct Posters {
    http: HttpClient,
    db: Db,
    dir: PathBuf,
    anilist: Arc<AniList>,
}

impl Posters {
    pub fn new(http: HttpClient, db: Db, dir: PathBuf, anilist: Arc<AniList>) -> Self {
        Self {
            http,
            db,
            dir,
            anilist,
        }
    }

    async fn download(&self, url: &str) -> Result<Vec<u8>, AppError> {
        let fail = |e: reqwest::Error| AppError::new("Blocked", format!("image {url}: {e}"));
        let resp = self
            .http
            .inner()
            .get(url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(fail)?;
        Ok(resp.bytes().await.map_err(fail)?.to_vec())
    }

    /// Saves an image under `<name>.<extension of the url>`.
    fn save(&self, name: &str, url: &str, bytes: &[u8]) -> Result<PathBuf, AppError> {
        let ext = url
            .rsplit('.')
            .next()
            .filter(|e| e.len() <= 4 && e.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or("jpg");
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(format!("{name}.{ext}"));
        std::fs::write(&path, bytes)?;
        Ok(path)
    }

    /// A small picture (a character, a voice actor) kept under `people/`, downloaded once; the
    /// app cannot show remote images, only files it serves itself.
    pub async fn image(&self, url: &str) -> Option<PathBuf> {
        let key = hex::encode(&Sha256::digest(url.as_bytes())[..8]);
        let dir = self.dir.join("people");
        if let Ok(entries) = std::fs::read_dir(&dir) {
            let found = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .find(|p| p.file_stem().is_some_and(|s| s == key.as_str()));
            if found.is_some() {
                return found;
            }
        }
        let bytes = self
            .download(url)
            .await
            .map_err(|e| tracing::debug!("{}", e.message))
            .ok()?;
        let ext = url
            .rsplit('.')
            .next()
            .filter(|e| e.len() <= 4 && e.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or("jpg");
        std::fs::create_dir_all(&dir).ok()?;
        let path = dir.join(format!("{key}.{ext}"));
        std::fs::write(&path, bytes).ok()?;
        Some(path)
    }

    /// Posters saved before they were shrunk on download, made `POSTER_MAX_W` wide in place; how
    /// many. Slow (a decode each): run it in the background.
    pub fn shrink_cached(&self) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        let mut done = 0;
        for path in entries.filter_map(|e| e.ok()).map(|e| e.path()) {
            let banner = path
                .file_stem()
                .is_some_and(|s| s.to_string_lossy().ends_with("-banner"));
            let wide = image::ImageReader::open(&path)
                .and_then(|r| r.with_guessed_format())
                .ok()
                .and_then(|r| r.into_dimensions().ok())
                .is_some_and(|(w, _)| w > POSTER_MAX_W);
            if !path.is_file() || banner || !wide {
                continue;
            }
            if let Some(small) = std::fs::read(&path)
                .ok()
                .and_then(|b| shrink(&b, POSTER_MAX_W))
            {
                // A new file put in place: a reader never sees half of one.
                let tmp = path.with_extension("shrinking");
                if std::fs::write(&tmp, small).is_ok() && std::fs::rename(&tmp, &path).is_ok() {
                    done += 1;
                }
            }
        }
        done
    }

    /// The wide AniList banner of a title, downloaded on first use; `None` when it has none.
    pub async fn banner(&self, anime: &Anime) -> Result<Option<PathBuf>, AppError> {
        let known: Option<String> = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT banner_path FROM titles WHERE shikimori_id = ?1",
                [anime.id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if let Some(path) = known.filter(|p| std::path::Path::new(p).exists()) {
            return Ok(Some(PathBuf::from(path)));
        }
        let Some(url) = self.anilist.art(anime.id).await.and_then(|a| a.banner) else {
            return Ok(None);
        };
        let bytes = self.download(&url).await?;
        let path = self.save(&format!("{}-banner", anime.id), &url, &bytes)?;
        self.db.lock().unwrap().execute(
            "UPDATE titles SET banner_path = ?1 WHERE shikimori_id = ?2",
            params![path.to_string_lossy(), anime.id],
        )?;
        Ok(Some(path))
    }

    /// The poster file and palette if they were downloaded before; never touches the network.
    pub fn cached(&self, id: i64) -> Result<Option<(PathBuf, Palette)>, AppError> {
        let known: Option<(Option<String>, Option<String>)> = {
            let conn = self.db.lock().unwrap();
            conn.query_row(
                "SELECT poster_path, palette_json FROM titles WHERE shikimori_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
        };
        Ok(match known {
            Some((Some(path), Some(palette))) if std::path::Path::new(&path).exists() => Some((
                PathBuf::from(path),
                serde_json::from_str(&palette).unwrap_or_default(),
            )),
            _ => None,
        })
    }

    /// Returns the cached poster file and its palette, downloading the poster on first use.
    /// The title must already be stored in the catalog cache.
    pub async fn ensure(&self, anime: &Anime) -> Result<Option<(PathBuf, Palette)>, AppError> {
        if let Some(found) = self.cached(anime.id)? {
            return Ok(Some(found));
        }
        // AniList covers are larger than Shikimori posters; Shikimori stays the fallback.
        let cover = self.anilist.art(anime.id).await.and_then(|a| a.cover);
        let Some(url) = cover.or_else(|| anime.poster_url.clone()) else {
            return Ok(None);
        };
        let bytes = self.download(&url).await?;
        let bytes = shrink(&bytes, POSTER_MAX_W).unwrap_or(bytes);
        let path = self.save(&anime.id.to_string(), &url, &bytes)?;
        let palette = image::load_from_memory(&bytes)
            .map(|img| pick_palette(&img))
            .unwrap_or_default();
        {
            let conn = self.db.lock().unwrap();
            conn.execute(
                "UPDATE titles SET poster_path = ?1, palette_json = ?2 WHERE shikimori_id = ?3",
                params![
                    path.to_string_lossy(),
                    serde_json::to_string(&palette)?,
                    anime.id
                ],
            )?;
        }
        Ok(Some((path, palette)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgb, RgbImage};

    fn hex_rgb(hex: &str) -> (u8, u8, u8) {
        let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap();
        ((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    #[test]
    fn palette_prefers_saturated_color_over_gray() {
        let mut img = RgbImage::new(2, 2);
        img.put_pixel(0, 0, Rgb([200, 30, 30]));
        img.put_pixel(1, 0, Rgb([190, 40, 35]));
        img.put_pixel(0, 1, Rgb([128, 128, 128]));
        img.put_pixel(1, 1, Rgb([120, 120, 120]));
        let p = pick_palette(&DynamicImage::ImageRgb8(img));
        let (r, g, b) = hex_rgb(&p.accent);
        assert!(
            r > 150 && g < 140 && b < 140,
            "accent should be red, got {}",
            p.accent
        );
        let (gr, _, _) = hex_rgb(&p.glow);
        assert!(gr < r, "glow is darker than accent");
    }

    /// A PNG of one flat color, standing in for a poster or a banner.
    fn png(rgb: [u8; 3]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(RgbImage::from_pixel(8, 12, Rgb(rgb)))
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[tokio::test]
    async fn posters_and_banners_come_from_anilist_when_it_has_them() {
        use axum::routing::{get, post};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let answer = serde_json::json!({"data": {"Page": {"media": [{"idMal": 52991,
            "bannerImage": format!("{base}/banner.png"),
            "coverImage": {"extraLarge": format!("{base}/anilist.png"), "color": null}}]}}})
        .to_string();
        let router = axum::Router::new()
            .route("/graphql", post(move || async move { answer }))
            .route("/anilist.png", get(|| async { png([200, 30, 30]) }))
            .route("/banner.png", get(|| async { png([30, 30, 200]) }))
            .route("/shikimori.png", get(|| async { png([30, 200, 30]) }));
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        let db: Db =
            std::sync::Arc::new(std::sync::Mutex::new(crate::db::open_in_memory().unwrap()));
        let http = HttpClient::new("ninari-test");
        let mut anime = crate::catalog::parse_animes(include_str!(
            "../../tests/fixtures/shikimori_search.json"
        ))
        .unwrap()
        .remove(0);
        anime.id = 52991;
        anime.poster_url = Some(format!("{base}/shikimori.png"));
        crate::catalog::Catalog::new(http.clone(), db.clone(), vec![])
            .store(&anime)
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let anilist = AniList::with_endpoint(http.clone(), db.clone(), &format!("{base}/graphql"));
        let posters = Posters::new(http, db, dir.path().to_path_buf(), anilist);

        let (path, _) = posters.ensure(&anime).await.unwrap().unwrap();
        assert_eq!(
            std::fs::read(path).unwrap(),
            png([200, 30, 30]),
            "AniList cover"
        );
        let banner = posters.banner(&anime).await.unwrap().unwrap();
        assert_eq!(std::fs::read(banner).unwrap(), png([30, 30, 200]));
    }

    #[tokio::test]
    async fn portraits_are_downloaded_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let router = axum::Router::new().route(
            "/face.jpg",
            axum::routing::get(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                async { png([10, 20, 30]) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/face.jpg", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        let db: Db =
            std::sync::Arc::new(std::sync::Mutex::new(crate::db::open_in_memory().unwrap()));
        let http = HttpClient::new("ninari-test");
        let dir = tempfile::tempdir().unwrap();
        let posters = Posters::new(
            http.clone(),
            db.clone(),
            dir.path().to_path_buf(),
            AniList::with_endpoint(http, db, "http://127.0.0.1:9/"),
        );
        let first = posters.image(&url).await.unwrap();
        let again = posters.image(&url).await.unwrap();
        assert_eq!(first, again);
        assert!(first.starts_with(dir.path()));
        assert_eq!(std::fs::read(&first).unwrap(), png([10, 20, 30]));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn wide_posters_shrink_and_narrow_ones_stay() {
        let encode = |w: u32, h: u32, format| {
            let mut out = std::io::Cursor::new(Vec::new());
            DynamicImage::ImageRgb8(RgbImage::new(w, h))
                .write_to(&mut out, format)
                .unwrap();
            out.into_inner()
        };
        let wide = encode(1900, 2700, image::ImageFormat::Jpeg);
        let small = shrink(&wide, POSTER_MAX_W).unwrap();
        let img = image::load_from_memory(&small).unwrap();
        assert_eq!((img.width(), img.height()), (440, 625));
        assert_eq!(
            image::guess_format(&small).unwrap(),
            image::ImageFormat::Jpeg
        );
        let png = shrink(&encode(900, 900, image::ImageFormat::Png), POSTER_MAX_W).unwrap();
        assert_eq!(image::guess_format(&png).unwrap(), image::ImageFormat::Png);
        assert!(shrink(&encode(400, 600, image::ImageFormat::Png), POSTER_MAX_W).is_none());
    }

    #[test]
    fn palette_falls_back_for_grayscale() {
        let img = RgbImage::from_pixel(4, 4, Rgb([90, 90, 90]));
        let p = pick_palette(&DynamicImage::ImageRgb8(img));
        assert_eq!(p.accent, DEFAULT_ACCENT);
    }
}
