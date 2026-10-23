//! Fonts and icons built into the binary.

use std::borrow::Cow;

use gpui::{App, AssetSource, Result, SharedString, Styled, Svg, px, svg};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets"]
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(Self::get(path).map(|f| f.data))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Self::iter()
            .filter(|p| p.starts_with(path))
            .map(SharedString::from)
            .collect())
    }
}

/// Manrope, the app's typeface, in the weights the screens use.
pub fn load_fonts(cx: &App) -> Result<()> {
    let fonts = Assets::iter()
        .filter(|p| p.starts_with("fonts/"))
        .filter_map(|p| Assets::get(&p).map(|f| f.data))
        .collect();
    cx.text_system().add_fonts(fonts)
}

/// One of the app's icons (assets/icons), drawn in the current text colour.
pub fn icon(name: &str, size: f32) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(size))
        .flex_none()
}
