//! Everything ninari does that needs no window: the catalog, sources and their plugins, the
//! library, sync with Shikimori and other devices, the network and the stream proxy. The GPUI app
//! in crates/app draws it.

pub mod account;
pub mod catalog;
pub mod db;
pub mod devices;
pub mod error;
pub mod extensions;
pub mod feeds;
pub mod library;
pub mod maintenance;
pub mod net;
pub mod player;
pub mod proxy;
pub mod sources;
pub mod stats;
pub mod sync;
pub mod watch;
