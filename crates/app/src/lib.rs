//! ninari's window, drawn by GPUI over ninari_core.

use gpui::{App, ClickEvent, Window};

pub mod assets;
pub mod button;
pub mod cards;
pub mod catalog;
pub mod catalog_page;
pub mod clock;
pub mod core;
pub mod data;
pub mod episode_grid;
pub mod episodes;
pub mod facts;
pub mod format;
pub mod franchise;
pub mod history_page;
pub mod home;
pub mod images;
pub mod lazy_grid;
pub mod library;
pub mod library_page;
pub mod modal;
pub mod motion;
// Video inside the window: libmpv in a Wayland surface. Elsewhere mpv plays in a window of its own.
#[cfg(target_os = "linux")]
pub mod mpv;
pub mod nav;
pub mod next_episode;
pub mod onboarding;
pub mod palette;
pub mod people_page;
pub mod pinned;
pub mod playback;
pub mod player;
pub mod player_panels;
pub mod popover;
pub mod posters;
pub mod rail;
pub mod repo_offer;
pub mod root;
pub mod schedule;
pub mod schedule_page;
pub mod scroll_row;
pub mod search_page;
pub mod season_cards;
pub mod season_switch;
pub mod select;
pub mod settings_data;
pub mod settings_devices;
pub mod settings_page;
pub mod settings_plugins;
pub mod settings_shikimori;
pub mod shell;
pub mod smooth;
pub mod source_fix;
pub mod stats_page;
pub mod text_input;
pub mod theme;
pub mod thumbs;
pub mod title;
pub mod title_about;
pub mod updates;
#[cfg(target_os = "linux")]
pub mod video;
#[cfg(windows)]
pub mod video_win;

pub type OnClick = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
