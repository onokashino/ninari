//! Draws a screen to a PNG without a window, for looking at the UI while developing without
//! opening anything on the desktop:
//!
//!   shot <out.png> [page] [width height] [step...]
//!
//! The page is a name from `Route::parse`; `SHOT_WAIT=<seconds>` waits longer for the network.
//! Steps then play the mouse and the keyboard, in logical pixels:
//!
//!   hover:X,Y  click:X,Y  drag:X1,Y1,X2,Y2  scroll:X,Y,DY  wheel:X,Y,NOTCHES  key:escape
//!   wait:MS  snap:<png>  bench:N (prints the CPU time of a frame)  clip:<png> (to the clipboard)
//!
//! `scroll` is a touchpad's exact pixels, `wheel` a mouse wheel's notches.
//!
//! `snap` saves a frame on the way; the last frame always goes to <out.png>.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use gpui::{
    AnyWindowHandle, AppContext as _, HeadlessAppContext, KeyDownEvent, Keystroke, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, PlatformInput, Point,
    ScrollDelta, ScrollEvent, ScrollSource, TouchPhase, point, px, size,
};
use ninari_app::assets::{Assets, load_fonts};
use ninari_app::nav::{Back, Forward, Route};
use ninari_app::player::Request;
use ninari_app::{core::Core, root::Root};
use ninari_core::catalog::posters::Palette;
use ninari_core::player::PlayerState;

/// Lets tasks, the network and animations run for `ms` of real time.
fn settle(cx: &mut HeadlessAppContext, window: AnyWindowHandle, ms: u64) -> anyhow::Result<()> {
    for _ in 0..ms.div_ceil(16).max(1) {
        cx.run_until_parked();
        cx.update_window(window, |_, window, _| window.refresh())?;
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(16));
    }
    Ok(())
}

fn send(
    cx: &mut HeadlessAppContext,
    window: AnyWindowHandle,
    input: PlatformInput,
) -> anyhow::Result<()> {
    cx.update_window(window, |_, window, cx| {
        window.dispatch_event(input, cx);
    })?;
    settle(cx, window, 16)
}

fn at(args: &str) -> anyhow::Result<Vec<Pixels>> {
    args.split(',')
        .map(|v| {
            v.trim()
                .parse::<f32>()
                .map(px)
                .with_context(|| format!("not a number: {v}"))
        })
        .collect()
}

fn mouse_move(position: Point<Pixels>, pressed: Option<MouseButton>) -> PlatformInput {
    PlatformInput::MouseMove(MouseMoveEvent {
        position,
        pressed_button: pressed,
        modifiers: Modifiers::default(),
    })
}

fn mouse_down(position: Point<Pixels>) -> PlatformInput {
    PlatformInput::MouseDown(MouseDownEvent {
        button: MouseButton::Left,
        position,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    })
}

fn mouse_up(position: Point<Pixels>) -> PlatformInput {
    PlatformInput::MouseUp(MouseUpEvent {
        button: MouseButton::Left,
        position,
        modifiers: Modifiers::default(),
        click_count: 1,
    })
}

fn step(cx: &mut HeadlessAppContext, window: AnyWindowHandle, step: &str) -> anyhow::Result<()> {
    let (name, args) = step.split_once(':').unwrap_or((step, ""));
    match name {
        "hover" => {
            let p = at(args)?;
            send(cx, window, mouse_move(point(p[0], p[1]), None))?;
        }
        "click" => {
            let p = at(args)?;
            let p = point(p[0], p[1]);
            send(cx, window, mouse_move(p, None))?;
            send(cx, window, mouse_down(p))?;
            send(cx, window, mouse_up(p))?;
        }
        "drag" => {
            let p = at(args)?;
            let (from, to) = (point(p[0], p[1]), point(p[2], p[3]));
            send(cx, window, mouse_move(from, None))?;
            send(cx, window, mouse_down(from))?;
            for i in 1..=12 {
                let t = i as f32 / 12.;
                let p = point(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
                send(cx, window, mouse_move(p, Some(MouseButton::Left)))?;
            }
            send(cx, window, mouse_up(to))?;
        }
        "scroll" => {
            let p = at(args)?;
            send(
                cx,
                window,
                PlatformInput::Scroll(ScrollEvent {
                    position: point(p[0], p[1]),
                    delta: ScrollDelta::Pixels(point(px(0.), -p[2])),
                    modifiers: Modifiers::default(),
                    touch_phase: TouchPhase::Moved,
                    source: ScrollSource::Pointer,
                }),
            )?;
        }
        "wheel" => {
            let p = at(args)?;
            send(
                cx,
                window,
                PlatformInput::Scroll(ScrollEvent {
                    position: point(p[0], p[1]),
                    delta: ScrollDelta::Lines(point(0., -f32::from(p[2]))),
                    modifiers: Modifiers::default(),
                    touch_phase: TouchPhase::Moved,
                    source: ScrollSource::Pointer,
                }),
            )?;
        }
        "key" => send(
            cx,
            window,
            PlatformInput::KeyDown(KeyDownEvent {
                keystroke: Keystroke::parse(args.to_string().leak())
                    .map_err(|_| anyhow::anyhow!("no key {args}"))?,
                is_held: false,
                prefer_character_input: false,
            }),
        )?,
        // A picture on the clipboard, as a screenshot tool leaves it.
        "clip" => {
            let bytes = std::fs::read(args)?;
            cx.update(|cx| {
                cx.write_to_clipboard(gpui::ClipboardItem {
                    entries: vec![gpui::ClipboardEntry::Image(gpui::Image::from_bytes(
                        gpui::ImageFormat::Png,
                        bytes,
                    ))],
                })
            });
        }
        "wait" => settle(cx, window, args.parse()?)?,
        "bench" => {
            // The CPU side of a frame: build, lay out and paint the whole window, N times.
            let mut times = Vec::new();
            for _ in 0..args.parse::<usize>()? {
                let took = cx.update_window(window, |_, window, cx| {
                    let start = std::time::Instant::now();
                    window.refresh();
                    window.draw(cx).clear(cx);
                    start.elapsed()
                })?;
                times.push(took.as_secs_f64() * 1000.);
                cx.run_until_parked();
            }
            times.sort_by(f64::total_cmp);
            let avg = times.iter().sum::<f64>() / times.len() as f64;
            println!(
                "frame: avg {avg:.2} ms, median {:.2} ms, max {:.2} ms",
                times[times.len() / 2],
                times[times.len() - 1]
            );
        }
        "snap" => {
            settle(cx, window, 16)?;
            cx.capture_screenshot(window)?.save(args)?;
            println!("{args}");
        }
        other => bail!("no step {other}"),
    }
    Ok(())
}

/// A believable moment of an episode, for the player screen without mpv.
fn sample_state(id: i64, paused: bool) -> (Request, PlayerState) {
    let request = Request {
        id,
        episode: 2,
        dub_key: Some("youkai".into()),
        start: None,
        title: "Киберпанк: Бегущие по краю 2".into(),
        palette: Palette::default(),
    };
    let state = PlayerState {
        shikimori_id: id,
        episode: 2,
        title: request.title.to_string(),
        dub_key: "youkai".into(),
        dub_name: "Дубляж Youkai Studio".into(),
        quality: Some(1080),
        extension_id: "yummy".into(),
        position: 140.,
        duration: 1708.,
        cached_until: 600.,
        paused,
        buffering: false,
        volume: 80.,
        muted: false,
        upscale: false,
        upscale_cfg: Default::default(),
        upscale_label: None,
        opening: Some((95., 185.)),
        ending: Some((1580., 1670.)),
        prev: Some(1),
        next: Some(3),
        fill: false,
        qualities: vec![2160, 1080, 720],
        quality_cap: None,
        audio_fx: Default::default(),
        audio_delay: 0.,
        audio_tracks: vec![],
        audio_track: None,
        speed: 1.,
        skip_opening: true,
        skip_ending: false,
        skip_fillers: false,
    };
    (request, state)
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1).peekable();
    let out = args.next().unwrap_or_else(|| "shot.png".into());
    let page = args.next_if(|a| {
        !a.contains(':')
            || a.starts_with("title:")
            || a.starts_with("character:")
            || a.starts_with("person:")
            || a.starts_with("player")
            || a.starts_with("search:")
    });
    let search = page
        .as_deref()
        .and_then(|p| p.strip_prefix("search:"))
        .map(str::to_string);
    // "player:<id>" and "player-paused:<id>" show the player screen over a made-up state.
    let preview = page.as_deref().and_then(|p| {
        let (kind, id) = p.split_once(':')?;
        Some((kind == "player-paused", id.parse::<i64>().ok()?))
            .filter(|_| kind.starts_with("player"))
    });
    // "onboarding" shows the first start's steps over the home screen.
    let onboarding = page.as_deref() == Some("onboarding");
    let route = match page.filter(|_| preview.is_none() && search.is_none() && !onboarding) {
        Some(name) => Route::parse(&name).ok_or_else(|| anyhow::anyhow!("no page {name}"))?,
        None => Route::Home,
    };
    let mut number = |default| {
        args.next_if(|a| a.parse::<f32>().is_ok())
            .map_or(default, |v| v.parse().unwrap_or(default))
    };
    let (w, h): (f32, f32) = (number(1440.), number(900.));
    let steps: Vec<String> = args.collect();

    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_writer(std::io::stderr)
        .init();
    let rt = tokio::runtime::Runtime::new()?;
    let core = Core::open(&rt)?;
    let text = Arc::new(gpui_wgpu::CosmicTextSystem::new("Manrope"));
    let mut cx = HeadlessAppContext::with_platform(
        text,
        Arc::new(Assets),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        core.install(cx);
        // The app's own keys, so `key:alt-left` goes back as it does there.
        ninari_app::text_input::bind_keys(cx);
        cx.bind_keys([
            gpui::KeyBinding::new("alt-left", Back, None),
            gpui::KeyBinding::new("alt-right", Forward, None),
        ]);
        load_fonts(cx)
    })?;
    let window = cx.open_window(size(px(w), px(h)), |window, cx| {
        cx.new(|cx| {
            let mut root = Root::new(core, false, window, cx);
            root.open(route, cx);
            if let Some(text) = &search {
                root.search_for(text, window, cx);
            }
            if onboarding {
                root.show_onboarding(window, cx);
            }
            if let Some((paused, id)) = preview {
                let (request, state) = sample_state(id, paused);
                root.preview_player(request, state, window, cx);
            }
            root
        })
    })?;
    let window: AnyWindowHandle = window.into();
    cx.allow_parking();
    // Long enough for the data and the posters a page asks for.
    let wait: u64 = std::env::var("SHOT_WAIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    settle(&mut cx, window, wait * 1000)?;
    for s in &steps {
        step(&mut cx, window, s).with_context(|| format!("step {s}"))?;
    }
    settle(&mut cx, window, 16)?;
    cx.capture_screenshot(window)?.save(&out)?;
    println!("{out}");
    Ok(())
}
