//! What sharpens a video smaller than the screen: Anime4K's chains (the official mpv modes,
//! https://github.com/bloc97/Anime4K/blob/master/md/GLSL_Instructions_Advanced.md) or
//! ArtCNN's luma doublers (https://github.com/Artoriuz/ArtCNN). Shaders live in the mpv config
//! folder (`scripts/fetch-shaders.sh`).

use serde::Serialize;

/// How heavy the networks are; `Off` keeps the picture as it is unless turned on by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Off,
    Fast,
    Hq,
    Ultra,
}

impl Tier {
    pub fn parse(s: &str) -> Self {
        match s {
            "off" => Tier::Off,
            "hq" => Tier::Hq,
            "ultra" => Tier::Ultra,
            _ => Tier::Fast,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Tier::Off => "off",
            Tier::Fast => "fast",
            Tier::Hq => "hq",
            Tier::Ultra => "ultra",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    Anime4k,
    Artcnn,
}

/// Anime4K's mode: A restores blurry, compressed video; B is gentler; C keeps clean sources as
/// they are and only upscales. `Auto` picks by the source's height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Auto,
    A,
    B,
    C,
}

/// ArtCNN's variant: neutral, trained to denoise and sharpen, or to denoise and soften.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Flavor {
    Neutral,
    Sharp,
    Soft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Upscale {
    pub method: Method,
    pub mode: Mode,
    pub tier: Tier,
    pub flavor: Flavor,
}

impl Default for Upscale {
    fn default() -> Self {
        Self {
            method: Method::Anime4k,
            mode: Mode::Auto,
            tier: Tier::Fast,
            flavor: Flavor::Neutral,
        }
    }
}

pub const METHODS: [(Method, &str); 2] = [(Method::Anime4k, "anime4k"), (Method::Artcnn, "artcnn")];
pub const MODES: [(Mode, &str); 4] = [
    (Mode::Auto, "auto"),
    (Mode::A, "a"),
    (Mode::B, "b"),
    (Mode::C, "c"),
];
pub const FLAVORS: [(Flavor, &str); 3] = [
    (Flavor::Neutral, "neutral"),
    (Flavor::Sharp, "sharp"),
    (Flavor::Soft, "soft"),
];

/// The value a settings key names in `table`.
pub fn by_key<T: Copy>(table: &[(T, &str)], key: &str) -> Option<T> {
    table.iter().find(|(_, k)| *k == key).map(|(v, _)| *v)
}

/// The settings key of a value in `table`.
pub fn key_of<T: Copy + PartialEq>(table: &[(T, &'static str)], value: T) -> &'static str {
    table
        .iter()
        .find(|(v, _)| *v == value)
        .map_or("", |(_, k)| *k)
}

/// Anime4K's chain as it runs for a source this tall: `Auto` resolved, and A doubled for
/// the weakest sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chain {
    A,
    AA,
    B,
    C,
}

pub fn chain(mode: Mode, source_height: u32) -> Chain {
    match mode {
        Mode::A => Chain::A,
        Mode::B => Chain::B,
        Mode::C => Chain::C,
        Mode::Auto if source_height >= 1000 => Chain::C,
        Mode::Auto if source_height >= 700 => Chain::A,
        Mode::Auto => Chain::AA,
    }
}

/// What runs, for the player's panel: "Anime4K A+A, HQ" or "ArtCNN, чётче".
pub fn label(u: &Upscale, source_height: u32) -> String {
    let tier = match u.tier {
        Tier::Off | Tier::Fast => "Fast",
        Tier::Hq => "HQ",
        Tier::Ultra => "Ultra",
    };
    match u.method {
        Method::Anime4k => {
            let chain = match chain(u.mode, source_height) {
                Chain::A => "A",
                Chain::AA => "A+A",
                Chain::B => "B",
                Chain::C => "C",
            };
            format!("Anime4K {chain}, {tier}")
        }
        Method::Artcnn => {
            let flavor = match u.flavor {
                Flavor::Neutral => "обычный",
                Flavor::Sharp => "чётче",
                Flavor::Soft => "мягче",
            };
            let size = if matches!(u.tier, Tier::Off | Tier::Fast) {
                "лёгкий"
            } else {
                "полный"
            };
            format!("ArtCNN {size}, {flavor}")
        }
    }
}

/// Shaders for a source of `source_height` on a screen of `screen_height`, or `None` when
/// upscaling is off or pointless.
pub fn shaders(source_height: u32, screen_height: u32, u: &Upscale) -> Option<Vec<String>> {
    if source_height == 0 || source_height >= screen_height || u.tier == Tier::Off {
        return None;
    }
    let names: Vec<String> = match u.method {
        Method::Anime4k => anime4k(chain(u.mode, source_height), u.tier),
        Method::Artcnn => {
            let size = if u.tier == Tier::Fast {
                "C4F16"
            } else {
                "C4F32"
            };
            let suffix = match u.flavor {
                Flavor::Neutral => "",
                Flavor::Sharp => "_DS",
                Flavor::Soft => "_DN",
            };
            vec![format!("ArtCNN_{size}{suffix}")]
        }
    };
    Some(
        names
            .into_iter()
            .map(|n| format!("~~/shaders/{n}.glsl"))
            .collect(),
    )
}

fn anime4k(chain: Chain, tier: Tier) -> Vec<String> {
    // Restore, first upscale, second upscale, and the lighter restore A+A runs between them.
    let (r, u1, u2, r2) = match tier {
        Tier::Off | Tier::Fast => ("M", "M", "S", "S"),
        Tier::Hq => ("VL", "VL", "M", "M"),
        Tier::Ultra => ("UL", "UL", "M", "M"),
    };
    let mut out = vec!["Anime4K_Clamp_Highlights".to_string()];
    match chain {
        Chain::A | Chain::AA => {
            out.push(format!("Anime4K_Restore_CNN_{r}"));
            out.push(format!("Anime4K_Upscale_CNN_x2_{u1}"));
            if chain == Chain::AA {
                out.push(format!("Anime4K_Restore_CNN_{r2}"));
            }
        }
        Chain::B => {
            out.push(format!("Anime4K_Restore_CNN_Soft_{r}"));
            out.push(format!("Anime4K_Upscale_CNN_x2_{u1}"));
        }
        Chain::C => out.push(format!("Anime4K_Upscale_Denoise_CNN_x2_{u1}")),
    }
    out.push("Anime4K_AutoDownscalePre_x2".into());
    out.push("Anime4K_AutoDownscalePre_x4".into());
    out.push(format!("Anime4K_Upscale_CNN_x2_{u2}"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(source: u32, screen: u32, u: Upscale) -> Vec<String> {
        shaders(source, screen, &u)
            .unwrap()
            .into_iter()
            .map(|p| {
                p.trim_start_matches("~~/shaders/")
                    .trim_end_matches(".glsl")
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn nothing_runs_when_off_pointless_or_unknown() {
        let u = Upscale::default();
        assert!(shaders(1080, 1080, &u).is_none(), "source fills the screen");
        assert!(shaders(0, 1080, &u).is_none(), "height not known yet");
        let off = Upscale {
            tier: Tier::Off,
            ..u
        };
        assert!(shaders(720, 1440, &off).is_none());
    }

    #[test]
    fn auto_picks_the_mode_by_the_source() {
        assert_eq!(chain(Mode::Auto, 1080), Chain::C);
        assert_eq!(chain(Mode::Auto, 720), Chain::A);
        assert_eq!(chain(Mode::Auto, 480), Chain::AA);
        assert_eq!(chain(Mode::B, 480), Chain::B);
    }

    #[test]
    fn anime4k_chains_follow_the_official_modes() {
        let hq = Upscale {
            tier: Tier::Hq,
            mode: Mode::A,
            ..Upscale::default()
        };
        assert_eq!(
            names(720, 1440, hq),
            [
                "Anime4K_Clamp_Highlights",
                "Anime4K_Restore_CNN_VL",
                "Anime4K_Upscale_CNN_x2_VL",
                "Anime4K_AutoDownscalePre_x2",
                "Anime4K_AutoDownscalePre_x4",
                "Anime4K_Upscale_CNN_x2_M"
            ]
        );
        let aa = names(480, 1440, Upscale::default());
        assert!(aa.contains(&"Anime4K_Restore_CNN_S".to_string()), "{aa:?}");
        let c = names(1080, 2160, Upscale::default());
        assert!(
            c.contains(&"Anime4K_Upscale_Denoise_CNN_x2_M".to_string()),
            "{c:?}"
        );
        assert!(!c.iter().any(|n| n.contains("Restore")), "{c:?}");
    }

    #[test]
    fn artcnn_is_one_doubler_by_size_and_flavor() {
        let art = |tier, flavor| Upscale {
            method: Method::Artcnn,
            tier,
            flavor,
            ..Upscale::default()
        };
        assert_eq!(
            names(720, 1440, art(Tier::Fast, Flavor::Neutral)),
            ["ArtCNN_C4F16"]
        );
        assert_eq!(
            names(720, 1440, art(Tier::Hq, Flavor::Sharp)),
            ["ArtCNN_C4F32_DS"]
        );
        assert_eq!(
            names(720, 1440, art(Tier::Ultra, Flavor::Soft)),
            ["ArtCNN_C4F32_DN"]
        );
    }

    #[test]
    fn labels_say_what_runs() {
        assert_eq!(label(&Upscale::default(), 480), "Anime4K A+A, Fast");
        let art = Upscale {
            method: Method::Artcnn,
            flavor: Flavor::Sharp,
            tier: Tier::Hq,
            ..Upscale::default()
        };
        assert_eq!(label(&art, 720), "ArtCNN полный, чётче");
    }

    #[test]
    fn tier_parses_settings_value() {
        assert_eq!(Tier::parse("hq"), Tier::Hq);
        assert_eq!(Tier::parse("ultra"), Tier::Ultra);
        assert_eq!(Tier::parse("off"), Tier::Off);
        assert_eq!(Tier::parse("что-то"), Tier::Fast);
    }
}
