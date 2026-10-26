//! A poster card drawn as one element. As nested divs a card was a dozen layout nodes, each with
//! its own handlers; a page has a hundred cards and lays them all out on every frame of a scroll.
//! This element is one node of a fixed size that paints the poster, its frame and chips itself,
//! and lays out only its two texts.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, BoxShadow, ClickEvent, Corners, DispatchPhase,
    Element, ElementId, FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla, IntoElement,
    LayoutId, MouseButton, MouseClickEvent, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ObjectFit, Pixels, Point, Resource, SharedString, Style, StyledText, TextOverflow,
    TextStyleRefinement, Window, fill, outline, point, px, rgb, rgba, size,
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
    texts: Vec<AnyElement>,
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

/// A text laid out at `origin`, at most `width` wide.
fn text(
    origin: Point<Pixels>,
    width: Pixels,
    style: TextStyleRefinement,
    text: StyledText,
    window: &mut Window,
    cx: &mut App,
) -> (AnyElement, gpui::Size<Pixels>) {
    window.with_text_style(Some(style), |window| {
        let mut el = text.into_any_element();
        let size = el.layout_as_root(
            size(AvailableSpace::Definite(width), AvailableSpace::MinContent),
            window,
            cx,
        );
        el.prepaint_at(origin, window, cx);
        (el, size)
    })
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
        let (title, title_size) = text(
            point(bounds.left(), below),
            width,
            TextStyleRefinement {
                font_size: Some(px(13.).into()),
                line_height: Some(px(17.).into()),
                font_weight: Some(FontWeight::BOLD),
                color: Some(title_color),
                line_clamp: Some(3),
                text_overflow: Some(TextOverflow::Truncate("…".into())),
                ..Default::default()
            },
            StyledText::new(self.title.clone()),
            window,
            cx,
        );
        texts.push(title);
        if !self.meta.is_empty() {
            let (meta, _) = text(
                point(bounds.left(), below + title_size.height + px(4.)),
                width,
                TextStyleRefinement {
                    font_size: Some(px(12.).into()),
                    color: Some(rgb(theme::TEXT_3).into()),
                    line_clamp: Some(2),
                    text_overflow: Some(TextOverflow::Truncate("…".into())),
                    ..Default::default()
                },
                StyledText::new(self.meta.clone()),
                window,
                cx,
            );
            texts.push(meta);
        }
        let chip_style = TextStyleRefinement {
            font_size: Some(px(12.).into()),
            line_height: Some(px(16.).into()),
            font_weight: Some(FontWeight::EXTRA_BOLD),
            ..Default::default()
        };
        if let Some(score) = &self.score {
            let star = "★".len();
            let styled = StyledText::new(score.clone()).with_highlights([(
                0..star,
                gpui::HighlightStyle {
                    color: Some(rgb(GOLD).into()),
                    ..Default::default()
                },
            )]);
            // Its width from the shaped line (cached) puts it in the top right corner.
            let w = chip_width(score, window);
            let origin = point(frame.right() - px(8. + 7.) - w, frame.top() + px(8. + 2.));
            let (chip, size) = text(
                origin,
                width,
                chip_style.clone().color_of(theme::TEXT),
                styled,
                window,
                cx,
            );
            score_pill = Some(pad(
                Bounds {
                    origin,
                    size: gpui::size(w, size.height),
                },
                7.,
                2.,
            ));
            texts.push(chip);
        }
        if let Some(badge) = &self.badge {
            let origin = point(frame.left() + px(8. + 9.), frame.top() + px(8. + 3.));
            let (chip, size) = text(
                origin,
                width,
                TextStyleRefinement {
                    color: Some(rgb(theme::ACCENT_CONTRAST).into()),
                    ..chip_style.clone()
                },
                StyledText::new(badge.clone()),
                window,
                cx,
            );
            let w = chip_width(badge, window);
            badge_pill = Some(pad(
                Bounds {
                    origin,
                    size: gpui::size(w, size.height),
                },
                9.,
                3.,
            ));
            texts.push(chip);
        }
        if let Some(initial) = &self.initial {
            let style = TextStyleRefinement {
                font_size: Some(px(40.).into()),
                font_weight: Some(FontWeight::EXTRA_BOLD),
                color: Some(rgb(theme::TEXT_3).into()),
                ..Default::default()
            };
            let (_, size) = text(
                Point::default(),
                width,
                style.clone(),
                StyledText::new(initial.clone()),
                window,
                cx,
            );
            let origin = frame.center() - point(size.width / 2., size.height / 2.);
            let (letter, _) = text(
                origin,
                width,
                style,
                StyledText::new(initial.clone()),
                window,
                cx,
            );
            texts.push(letter);
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
        for text in &mut laid.texts {
            text.paint(window, cx);
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

/// How wide a chip's text is: a text element takes all the width it is offered, the pill under
/// it must not.
fn chip_width(text: &SharedString, window: &mut Window) -> Pixels {
    let run = gpui::TextRun {
        len: text.len(),
        font: gpui::Font {
            weight: FontWeight::EXTRA_BOLD,
            ..gpui::font(theme::FONT)
        },
        color: rgb(theme::TEXT).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text.clone(), px(12.), &[run], None)
        .width
}
