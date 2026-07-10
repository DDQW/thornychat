//! Fullscreen lightbox image. The whole picture scales with the mouse wheel
//! — growing past the viewport so you can zoom in far, not cropped inside a
//! fixed frame — and a zoomed-in image can be dragged to pan.
//!
//! The reason this exists instead of `iced::widget::image::Viewer`: `Viewer`
//! either clips the zoom to a small fixed frame (sized `Shrink`) or, sized to
//! fill, swallows *every* click inside its bounds. We need both "the image
//! grows to fill the space and beyond" and "a click on the empty margin
//! around it closes the lightbox". So this widget fills the whole layer, but a
//! left-press that lands on the translucent margin (not on the picture) is
//! deliberately returned as `Ignored` — that lets the backdrop `mouse_area`
//! wrapping it fire its `CloseZoom`. Only presses on the picture itself are
//! captured (to start a pan). Wheel zoom and pan never bubble.

use iced::advanced::image::{self, Image};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{mouse, Clipboard, Shell, Widget};
use iced::{ContentFit, Element, Event, Length, Point, Radians, Rectangle, Size, Vector};

/// Zoom level (relative to contain-fit, so 3.0 = "300%") past which the source
/// is magnified enough that super-resolution is worth requesting. The widget
/// publishes `on_upscale` once when the wheel first crosses this.
const UPSCALE_TRIGGER_SCALE: f32 = 3.0;

/// A raster image shown fullscreen with wheel-zoom and drag-to-pan. Always
/// lays out to fill the space it's given; the picture is contain-fit inside
/// that at rest (`scale` 1.0) and grows from there.
pub struct LightboxImage<Message, Handle> {
    handle: Handle,
    min_scale: f32,
    max_scale: f32,
    scale_step: f32,
    on_double_click: Option<Message>,
    on_upscale: Option<Message>,
}

impl<Message, Handle> LightboxImage<Message, Handle> {
    pub fn new(handle: Handle) -> Self {
        Self {
            handle,
            // 1.0 = contain-fit; wheel-down stops there rather than shrinking
            // the picture into a thumbnail. 20x gives plenty of "zoom in far".
            min_scale: 1.0,
            max_scale: 20.0,
            scale_step: 0.25,
            on_double_click: None,
            on_upscale: None,
        }
    }

    /// Message to publish when the picture itself is double-clicked — used to
    /// close the lightbox (a *single* click on the picture pans instead, so it
    /// can't double as the close gesture).
    pub fn on_double_click(mut self, message: Message) -> Self {
        self.on_double_click = Some(message);
        self
    }

    /// Message to publish once, the first time zoom passes
    /// [`UPSCALE_TRIGGER_SCALE`] — the caller uses it to kick off a
    /// super-resolution pass for the image being magnified.
    pub fn on_upscale(mut self, message: Message) -> Self {
        self.on_upscale = Some(message);
        self
    }
}

#[derive(Debug, Clone, Copy)]
struct State {
    /// Multiplier on the contain-fit size. 1.0 = fits the space at rest.
    scale: f32,
    /// Pan translation of the picture from centered, in pixels.
    offset: Vector,
    /// Cursor position where a pan-drag began, and the offset at that moment.
    grabbed_at: Option<Point>,
    grab_start_offset: Vector,
    /// Last press on the picture, for single/double-click discrimination.
    last_click: Option<mouse::Click>,
    /// Set once the upscale trigger has fired, so it fires exactly once per
    /// opened image (a fresh lightbox = a fresh `State`).
    upscale_signaled: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            scale: 1.0,
            offset: Vector::new(0.0, 0.0),
            grabbed_at: None,
            grab_start_offset: Vector::new(0.0, 0.0),
            last_click: None,
            upscale_signaled: false,
        }
    }
}

/// Contain-fit size of the image within `bounds` at scale 1.0. `Size::ZERO`
/// if the image hasn't been measured yet (avoids a divide-by-zero in `fit`).
fn fitted_size<Renderer>(renderer: &Renderer, handle: &Renderer::Handle, bounds: Size) -> Size
where
    Renderer: image::Renderer,
{
    let Some(measured) = renderer.measure_image(handle) else {
        return Size::ZERO;
    };
    if measured.width == 0 || measured.height == 0 || bounds.width <= 0.0 || bounds.height <= 0.0 {
        return Size::ZERO;
    }
    let image_size = Size::new(measured.width as f32, measured.height as f32);
    ContentFit::Contain.fit(image_size, bounds)
}

/// Clamps a pan offset so the picture can't be dragged off past its own edge:
/// no pan at all while it's smaller than the viewport, and only within the
/// overflow once it's larger.
fn clamp_offset(offset: Vector, scaled: Size, bounds: Size) -> Vector {
    let max_x = ((scaled.width - bounds.width) / 2.0).max(0.0);
    let max_y = ((scaled.height - bounds.height) / 2.0).max(0.0);
    Vector::new(offset.x.clamp(-max_x, max_x), offset.y.clamp(-max_y, max_y))
}

/// The picture's on-screen rectangle for a given scaled size and pan offset,
/// centered in `bounds`.
fn image_rect(scaled: Size, offset: Vector, bounds: Rectangle) -> Rectangle {
    let center = bounds.center();
    Rectangle {
        x: center.x - scaled.width / 2.0 + offset.x,
        y: center.y - scaled.height / 2.0 + offset.y,
        width: scaled.width,
        height: scaled.height,
    }
}

impl<Message, Theme, Renderer, Handle> Widget<Message, Theme, Renderer>
    for LightboxImage<Message, Handle>
where
    Message: Clone,
    Renderer: image::Renderer<Handle = Handle>,
    Handle: Clone,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size { width: Length::Fill, height: Length::Fill }
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.max())
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let state = tree.state.downcast_mut::<State>();

        match event {
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let Some(cursor_position) = cursor.position_over(bounds) else {
                    return;
                };
                let y = match delta {
                    mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => *y,
                };
                if y == 0.0 {
                    return;
                }
                // Direction-only step: a trackpad's many tiny deltas zoom the
                // same as a mouse wheel's notches.
                let factor =
                    if y > 0.0 { 1.0 + self.scale_step } else { 1.0 / (1.0 + self.scale_step) };
                let previous = state.scale;
                state.scale = (previous * factor).clamp(self.min_scale, self.max_scale);
                let ratio = state.scale / previous;
                if ratio != 1.0 {
                    // Anchor the zoom on the cursor: keep whatever image point
                    // is under the pointer pinned there as it scales, so the
                    // view zooms *into* the cursor rather than the center.
                    // Derived against `image_rect`'s centering — for cursor
                    // offset-from-center `d` and scale ratio `r`, the pan that
                    // holds that point fixed is `(1 - r)·d + r·offset`.
                    let d = cursor_position - bounds.center();
                    let offset = Vector::new(
                        (1.0 - ratio) * d.x + ratio * state.offset.x,
                        (1.0 - ratio) * d.y + ratio * state.offset.y,
                    );
                    let fitted = fitted_size::<Renderer>(renderer, &self.handle, bounds.size());
                    let scaled = Size::new(fitted.width * state.scale, fitted.height * state.scale);
                    state.offset = clamp_offset(offset, scaled, bounds.size());
                }
                // Once past the threshold, ask the caller (once) to
                // super-resolve the image being magnified.
                if !state.upscale_signaled && state.scale >= UPSCALE_TRIGGER_SCALE {
                    state.upscale_signaled = true;
                    if let Some(message) = &self.on_upscale {
                        shell.publish(message.clone());
                    }
                }
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(position) = cursor.position_over(bounds) else {
                    return;
                };
                let fitted = fitted_size::<Renderer>(renderer, &self.handle, bounds.size());
                let scaled = Size::new(fitted.width * state.scale, fitted.height * state.scale);
                let rect = image_rect(scaled, state.offset, bounds);
                if rect.contains(position) {
                    // On the picture. A double-click here closes (published to
                    // the caller); a single press starts a pan and is consumed
                    // so the backdrop doesn't close.
                    let click = mouse::Click::new(position, mouse::Button::Left, state.last_click);
                    state.last_click = Some(click);
                    if matches!(click.kind(), mouse::click::Kind::Double) {
                        state.grabbed_at = None;
                        if let Some(message) = &self.on_double_click {
                            shell.publish(message.clone());
                        }
                        shell.capture_event();
                        return;
                    }
                    state.grabbed_at = Some(position);
                    state.grab_start_offset = state.offset;
                    shell.capture_event();
                }
                // On the empty margin: leave it uncaptured for the backdrop's
                // click-to-close.
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.grabbed_at.take().is_some() {
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let Some(origin) = state.grabbed_at {
                    let fitted = fitted_size::<Renderer>(renderer, &self.handle, bounds.size());
                    let scaled = Size::new(fitted.width * state.scale, fitted.height * state.scale);
                    let delta = *position - origin;
                    state.offset =
                        clamp_offset(state.grab_start_offset + delta, scaled, bounds.size());
                    shell.capture_event();
                }
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();
        if state.grabbed_at.is_some() {
            return mouse::Interaction::Grabbing;
        }
        let bounds = layout.bounds();
        if let Some(position) = cursor.position_over(bounds) {
            let fitted = fitted_size::<Renderer>(renderer, &self.handle, bounds.size());
            let scaled = Size::new(fitted.width * state.scale, fitted.height * state.scale);
            // Only hint "grab" when there's actually overflow to pan; a click
            // on a not-yet-zoomed picture does nothing, so leave the
            // backdrop's pointer.
            if (scaled.width > bounds.width || scaled.height > bounds.height)
                && image_rect(scaled, state.offset, bounds).contains(position)
            {
                return mouse::Interaction::Grab;
            }
        }
        mouse::Interaction::None
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let state = tree.state.downcast_ref::<State>();
        let fitted = fitted_size::<Renderer>(renderer, &self.handle, bounds.size());
        if fitted.width <= 0.0 || fitted.height <= 0.0 {
            return;
        }
        let scaled = Size::new(fitted.width * state.scale, fitted.height * state.scale);
        let offset = clamp_offset(state.offset, scaled, bounds.size());
        let rect = image_rect(scaled, offset, bounds);

        // Clip to the widget's own bounds so a zoomed-in picture that overflows
        // the layer is cropped at the edges instead of drawing over everything.
        renderer.with_layer(bounds, |renderer| {
            renderer.draw_image(
                Image {
                    handle: self.handle.clone(),
                    filter_method: image::FilterMethod::default(),
                    rotation: Radians(0.0),
                    opacity: 1.0,
                    snap: true,
                    border_radius: Default::default(),
                },
                rect,
                bounds,
            );
        });
    }
}

impl<'a, Message, Theme, Renderer, Handle> From<LightboxImage<Message, Handle>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a + Clone,
    Renderer: 'a + image::Renderer<Handle = Handle>,
    Theme: 'a,
    Handle: Clone + 'a,
{
    fn from(widget: LightboxImage<Message, Handle>) -> Self {
        Element::new(widget)
    }
}
