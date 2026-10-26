//! A poster card drawn as one element. As nested divs a card was a dozen layout nodes, each with
//! its own handlers; a page has a hundred cards and lays them all out on every frame of a scroll.
//! This element is one node of a fixed size that paints the poster, its frame and chips itself,
//! and lays out only its two texts.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Bounds, BoxShadow, ClickEvent, Corners, DispatchPhase, Element, ElementId, FontWeight,
    GlobalElementId, Hitbox, HitboxBehavior, Hsla, IntoElement, LayoutId, MouseButton,
    MouseClickEvent, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Point,
    Resource, SharedString, Style, TextStyleRefinement, Window, WrappedLine, fill, outline, point,
    px, rgb, rgba, size,
};

use crate::OnClick;
use crate::hover_card;
use crate::images::Current;
use crate::motion;
use crate::theme::{self, SPRING};

/// What a poster card shows, already reduced to what is painted.
pub struct PosterCard {
    pub key: ElementId,
    pub width: f32,
    /// The whole card's height: the poster and room for the texts under it.
    pub height: f32,
    pub lift: f32,
    pub accent: Hsla,
    pub poster: Option<PathBuf>,
    /// The letter shown when the title has no poster at all.
    pub initial: Option<SharedString>,
    pub title: SharedString,
    pub meta: SharedString,
    pub score: Option<SharedString>,
    pub badge: Option<SharedString>,
    pub fraction: Option<f32>,
    pub on_click: Option<OnClick>,
}

pub struct Laid {
    hitbox: Hitbox,
    texts: Vec<(Point<Pixels>, Rc<Shaped>)>,
    /// The pills under the score and the badge.
    score_pill: Option<Bounds<Pixels>>,
    badge_pill: Option<Bounds<Pixels>>,
}

impl IntoElement for PosterCard {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

const RADIUS: f32 = 12.;
const GOLD: u32 = 0xf5c451;

/// A text shaped, wrapped and cut to its lines once: a card's title is the same on every frame of
/// a scroll, and laying it out again each time (a layout tree, cutting it by characters) was most
/// of what a card cost. Kept per text, width and look.
struct Shaped {
    lines: Vec<WrappedLine>,
    line_height: Pixels,
    size: gpui::Size<Pixels>,
}

thread_local! {
    static SHAPED: RefCell<HashMap<u64, Rc<Shaped>>> = RefCell::default();
}

/// `text` at most `width` wide in `style`; with `gold`, its first character (the score's star)
/// in gold.
fn text(
    width: Pixels,
    style: &TextStyleRefinement,
    text: &SharedString,
    gold: bool,
    window: &mut Window,
    cx: &mut App,
) -> Rc<Shaped> {
    let mut ts = window.text_style();
    gpui::Refineable::refine(&mut ts, style);
    let font_size = ts.font_size.to_pixels(window.rem_size());
    let key = {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        f32::from(width).to_bits().hash(&mut h);
        f32::from(font_size).to_bits().hash(&mut h);
        ts.font_family.hash(&mut h);
        ts.font_weight.0.to_bits().hash(&mut h);
        let c = ts.color;
        [c.h, c.s, c.l, c.a].map(f32::to_bits).hash(&mut h);
        ts.line_clamp.hash(&mut h);
        gold.hash(&mut h);
        h.finish()
    };
    if let Some(found) = SHAPED.with(|m| m.borrow().get(&key).cloned()) {
        return found;
    }
    let line_height = window.pixel_snap(
        ts.line_height
            .to_pixels(font_size.into(), window.rem_size()),
    );
    let star = text.chars().next().map_or(0, char::len_utf8);
    let runs = if gold && star < text.len() {
        vec![
            ts.clone()
                .highlight(gpui::HighlightStyle {
                    color: Some(rgb(GOLD).into()),
                    ..Default::default()
                })
                .to_run(star),
            ts.to_run(text.len() - star),
        ]
    } else {
        vec![ts.to_run(text.len())]
    };
    let (cut, runs) = match ts.line_clamp {
        Some(max) => {
            let (cut, runs) = cx
                .text_system()
                .line_wrapper(ts.font(), font_size)
                .truncate_wrapped_line(
                    text.clone(),
                    width,
                    max,
                    "…",
                    &runs,
                    gpui::TruncateFrom::End,
                );
            (cut, runs.into_owned())
        }
        None => (text.clone(), runs),
    };
    let lines = window
        .text_system()
        .shape_text(cut, font_size, &runs, Some(width), ts.line_clamp)
        .map(|lines| lines.into_iter().collect::<Vec<_>>())
        .unwrap_or_default();
    let mut size = gpui::Size::<Pixels>::default();
    for line in &lines {
        let s = line.size(line_height);
        size.height += s.height;
        size.width = size.width.max(s.width).ceil();
    }
    let shaped = Rc::new(Shaped {
        lines,
        line_height,
        size,
    });
    SHAPED.with(|m| {
        let mut m = m.borrow_mut();
        // Hover tints a title for a moment: those looks do not pile up for ever.
        if m.len() > 4096 {
            m.clear();
        }
        m.insert(key, shaped.clone());
    });
    shaped
}

fn paint_text(origin: Point<Pixels>, shaped: &Shaped, window: &mut Window, cx: &mut App) {
    let mut y = origin.y;
    for line in &shaped.lines {
        line.paint(
            point(origin.x, y),
            shaped.line_height,
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        )
        .ok();
        y += line.size(shaped.line_height).height;
    }
}

impl PosterCard {
    fn frame(&self, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
        Bounds {
            origin: point(bounds.left(), bounds.top() - px(6. * self.lift)),
            size: size(px(self.width), px(self.width * 1.5)),
        }
    }
}

impl Element for PosterCard {
    type RequestLayoutState = ();
    type PrepaintState = Laid;

    fn id(&self) -> Option<ElementId> {
        Some(self.key.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size = size(px(self.width).into(), px(self.height).into());
        style.flex_shrink = 0.;
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Laid {
        hover_card::place(&self.key, bounds, cx);
        let width = px(self.width);
        let frame = self.frame(bounds);
        let mut texts = Vec::new();
        let (mut score_pill, mut badge_pill) = (None, None);
        let title_color = theme::mix(
            rgb(theme::TEXT),
            self.accent,
            0.45 * self.lift.clamp(0., 1.),
        );
        let below = bounds.top() + px(self.width * 1.5 + 8.);
        let title = text(
            width,
            &TextStyleRefinement {
                font_size: Some(px(13.).into()),
                line_height: Some(px(17.).into()),
                font_weight: Some(FontWeight::BOLD),
                color: Some(title_color),
                line_clamp: Some(3),
                ..Default::default()
            },
            &self.title,
            false,
            window,
            cx,
        );
        let title_height = title.size.height;
        texts.push((point(bounds.left(), below), title));
        if !self.meta.is_empty() {
            let meta = text(
                width,
                &TextStyleRefinement {
                    font_size: Some(px(12.).into()),
                    color: Some(rgb(theme::TEXT_3).into()),
                    line_clamp: Some(2),
                    ..Default::default()
                },
                &self.meta,
                false,
                window,
                cx,
            );
            texts.push((point(bounds.left(), below + title_height + px(4.)), meta));
        }
        let chip_style = TextStyleRefinement {
            font_size: Some(px(12.).into()),
            line_height: Some(px(16.).into()),
            font_weight: Some(FontWeight::EXTRA_BOLD),
            ..Default::default()
        };
        if let Some(score) = &self.score {
            let chip = text(
                width,
                &chip_style.clone().color_of(theme::TEXT),
                score,
                true,
                window,
                cx,
            );
            // Its shaped width puts it in the top right corner.
            let w = chip.size.width;
            let origin = point(frame.right() - px(8. + 7.) - w, frame.top() + px(8. + 2.));
            score_pill = Some(pad(
                Bounds {
                    origin,
                    size: chip.size,
                },
                7.,
                2.,
            ));
            texts.push((origin, chip));
        }
        if let Some(badge) = &self.badge {
            let origin = point(frame.left() + px(8. + 9.), frame.top() + px(8. + 3.));
            let chip = text(
                width,
                &TextStyleRefinement {
                    color: Some(rgb(theme::ACCENT_CONTRAST).into()),
                    ..chip_style.clone()
                },
                badge,
                false,
                window,
                cx,
            );
            badge_pill = Some(pad(
                Bounds {
                    origin,
                    size: chip.size,
                },
                9.,
                3.,
            ));
            texts.push((origin, chip));
        }
        if let Some(initial) = &self.initial {
            let letter = text(
                width,
                &TextStyleRefinement {
                    font_size: Some(px(40.).into()),
                    font_weight: Some(FontWeight::EXTRA_BOLD),
                    color: Some(rgb(theme::TEXT_3).into()),
                    ..Default::default()
                },
                initial,
                false,
                window,
                cx,
            );
            let origin = frame.center() - point(letter.size.width / 2., letter.size.height / 2.);
            texts.push((origin, letter));
        }
        Laid {
            hitbox: window.insert_hitbox(bounds, HitboxBehavior::Normal),
            texts,
            score_pill,
            badge_pill,
        }
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        laid: &mut Laid,
        window: &mut Window,
        cx: &mut App,
    ) {
        let frame = self.frame(bounds);
        let radius = Corners::all(px(RADIUS));
        if self.lift > 0.01 {
            window.paint_drop_shadows(
                frame,
                radius,
                &[BoxShadow {
                    color: self.accent.opacity(0.6 * self.lift.clamp(0., 1.)),
                    offset: point(px(0.), px(16.)),
                    blur_radius: px(32.),
                    spread_radius: px(-14.),
                    inset: false,
                }],
            );
        }
        window.paint_quad(fill(frame, rgb(theme::SURFACE_2)).corner_radii(radius));
        if let Some(path) = &self.poster
            && let Some(images) = cx.try_global::<Current>().map(|c| c.0.clone())
        {
            let resource = Resource::Path(path.as_path().into());
            let data = images.update(cx, |images, cx| {
                gpui::ImageCache::load(images, &resource, window, cx)
            });
            if let Some(Ok(data)) = data {
                let fitted = ObjectFit::Cover.get_bounds(frame, data.size(0));
                window
                    .paint_image(frame, fitted, radius, data, 0, false)
                    .ok();
            }
        }
        window.paint_quad(
            outline(frame, theme::line(), gpui::BorderStyle::Solid).corner_radii(radius),
        );
        // The chips' pills, under their texts; a radius past half the height draws nothing.
        if let Some(b) = laid.score_pill {
            window.paint_quad(
                fill(b, rgba(0x0c0c12cc)).corner_radii(Corners::all(b.size.height / 2.)),
            );
        }
        if let Some(b) = laid.badge_pill {
            window.paint_quad(fill(b, self.accent).corner_radii(Corners::all(b.size.height / 2.)));
        }
        if let Some(f) = self.fraction.filter(|f| *f > 0.) {
            let bar = Bounds {
                origin: point(frame.left() + px(8.), frame.bottom() - px(12.)),
                size: size(px((f.min(1.) * self.width).min(self.width - 16.)), px(4.)),
            };
            window.paint_quad(fill(bar, self.accent).corner_radii(Corners::all(px(2.))));
        }
        for (origin, shaped) in &laid.texts {
            paint_text(*origin, shaped, window, cx);
        }

        let hitbox = laid.hitbox.clone();
        window.set_cursor_style(gpui::CursorStyle::PointingHand, &hitbox);
        let hovered = hitbox.is_hovered(window);
        let key = self.key.clone();
        // The page scrolled the card from under a still pointer: no move tells it, so it settles
        // down here.
        if !hovered && motion::is_on(&key, cx) {
            motion::set(key.clone(), false, Duration::from_millis(380), SPRING, cx);
            hover_card::hover(&key, false, cx);
        }
        {
            let hitbox = hitbox.clone();
            window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let now = hitbox.is_hovered(window);
                if now != hovered {
                    motion::set(key.clone(), now, Duration::from_millis(380), SPRING, cx);
                    hover_card::hover(&key, now, cx);
                    window.refresh();
                }
            });
        }
        let Some(on_click) = self.on_click.take() else {
            return;
        };
        let pressed = window.with_element_state::<Rc<RefCell<Option<MouseDownEvent>>>, _>(
            global_id.expect("a card has an id"),
            |state, _| {
                let state = state.unwrap_or_default();
                (state.clone(), state)
            },
        );
        let on_click = Rc::new(on_click);
        {
            let (pressed, hitbox) = (pressed.clone(), hitbox.clone());
            window.on_mouse_event(move |e: &MouseDownEvent, phase, window, _| {
                if phase == DispatchPhase::Bubble
                    && e.button == MouseButton::Left
                    && hitbox.is_hovered(window)
                {
                    *pressed.borrow_mut() = Some(e.clone());
                }
            });
        }
        let clicked = Rc::new(Cell::new(false));
        window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || e.button != MouseButton::Left {
                return;
            }
            let down = pressed.borrow_mut().take();
            if let Some(down) = down
                && hitbox.is_hovered(window)
                && !clicked.replace(true)
            {
                on_click(
                    &ClickEvent::Mouse(MouseClickEvent {
                        down,
                        up: e.clone(),
                    }),
                    window,
                    cx,
                );
            }
        });
    }
}

fn pad(b: Bounds<Pixels>, x: f32, y: f32) -> Bounds<Pixels> {
    Bounds {
        origin: b.origin - point(px(x), px(y)),
        size: b.size + size(px(2. * x), px(2. * y)),
    }
}

trait ColorOf {
    fn color_of(self, color: u32) -> Self;
}

impl ColorOf for TextStyleRefinement {
    fn color_of(mut self, color: u32) -> Self {
        self.color = Some(rgb(color).into());
        self
    }
}
