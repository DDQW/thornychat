//! Animated-GIF widget for custom emoji, stickers, and GIF image messages
//! (iced's core image widget draws only the first frame).
//!
//! This is a vendored, minimally-corrected copy of the `iced_gif` 0.13 widget
//! (MIT — <https://crates.io/crates/iced_gif>). The one behavioural change fixes
//! a duplication bug: upstream's `diff()` decides whether a *different* GIF now
//! occupies a reused widget-tree slot by comparing `ImageDecoder::total_bytes()`,
//! which for a GIF is essentially one frame's `width × height × 4`. Emoji packs
//! normalise their artwork to the same dimensions, so distinct animated emotes
//! share that value — iced then keeps showing the *previous* emote's cached
//! frame, and a distinct emoji renders as a duplicate of another (worst in the
//! reflowing picker grid). We key change-detection on a content hash of the raw
//! GIF bytes instead, which is unique per image. Layout and paint are delegated
//! to iced's own `image::{layout, draw}` helpers so there's no rendering logic
//! to keep in sync with the core widget.

use std::hash::Hasher;
use std::time::{Duration, Instant};

use iced::advanced::image::{self, FilterMethod, Handle};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{Clipboard, Shell, Widget};
use iced::{mouse, window, ContentFit, Element, Event, Length, Rectangle, Rotation, Size};

/// A decoded animated GIF: its frames plus a content-derived `id` used to tell
/// one GIF from another when a widget-tree slot is reused across re-renders.
pub struct Frames {
    id: u64,
    first: Frame,
    frames: Vec<Frame>,
}

impl std::fmt::Debug for Frames {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frames").field("id", &self.id).field("frames", &self.frames.len()).finish()
    }
}

impl Frames {
    /// Decode animated [`Frames`] from raw GIF bytes. `None` on any decode
    /// failure — the caller falls back to a static raster handle.
    pub fn from_bytes(bytes: Vec<u8>) -> Option<Self> {
        // The decode trait (`into_frames`) is on the `image` crate; refer to it
        // by its crate-root path so it isn't shadowed by the `iced::advanced::image`
        // import above.
        use ::image::AnimationDecoder;

        // Content hash → stable identity for `diff`. Unique per distinct GIF,
        // unlike upstream's frame byte-size (which collides for same-size
        // emotes and caused the duplication this module fixes).
        let id = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            hasher.write(&bytes);
            hasher.finish()
        };

        let decoder =
            ::image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).ok()?;
        let frames = decoder
            .into_frames()
            .map(|result| result.map(Frame::from))
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        let first = frames.first().cloned()?;

        Some(Frames { id, first, frames })
    }

    /// Content-hash identity used for widget change-detection — log this
    /// alongside the originating URL when diagnosing a "wrong image shown"
    /// report; two different URLs producing the same id here would mean a
    /// genuine hash collision (vanishingly unlikely) rather than a widget bug.
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn first_frame_size(&self) -> (u32, u32) {
        (self.first.width, self.first.height)
    }
}

#[derive(Clone)]
struct Frame {
    delay: Duration,
    handle: Handle,
    width: u32,
    height: u32,
}

impl From<::image::Frame> for Frame {
    fn from(frame: ::image::Frame) -> Self {
        let (width, height) = frame.buffer().dimensions();
        let delay: Duration = frame.delay().into();
        let handle = Handle::from_rgba(width, height, frame.into_buffer().into_raw());
        Self { delay, handle, width, height }
    }
}

struct State {
    id: u64,
    index: usize,
    current: Current,
}

struct Current {
    frame: Frame,
    started: Instant,
}

impl From<Frame> for Current {
    fn from(frame: Frame) -> Self {
        Self { started: Instant::now(), frame }
    }
}

/// Displays an animated GIF, advancing frames on the window's redraw ticks.
pub struct Gif<'a> {
    frames: &'a Frames,
    width: Length,
    height: Length,
    content_fit: ContentFit,
    filter_method: FilterMethod,
    rotation: Rotation,
    opacity: f32,
    /// The mxc URL this instance is showing, purely for the mismatch logs
    /// below — never read for rendering. Empty when the caller doesn't set
    /// one via `.debug_label(...)`.
    debug_label: &'a str,
}

/// Creates a [`Gif`] widget for the given decoded [`Frames`].
pub fn gif(frames: &Frames) -> Gif<'_> {
    Gif::new(frames)
}

impl<'a> Gif<'a> {
    pub fn new(frames: &'a Frames) -> Self {
        Self {
            frames,
            width: Length::Shrink,
            height: Length::Shrink,
            content_fit: ContentFit::Contain,
            filter_method: FilterMethod::default(),
            rotation: Rotation::default(),
            opacity: 1.0,
            debug_label: "",
        }
    }

    /// Sets the width of the [`Gif`] boundaries.
    pub fn width(mut self, width: Length) -> Self {
        self.width = width;
        self
    }

    /// Sets the height of the [`Gif`] boundaries.
    pub fn height(mut self, height: Length) -> Self {
        self.height = height;
        self
    }

    /// Tags this instance with its originating mxc URL, so a widget-slot
    /// mismatch (see `diff`/`on_event`/`draw` below) logs *which* emote's
    /// cell was affected instead of just an opaque content-hash pair.
    pub fn debug_label(mut self, label: &'a str) -> Self {
        self.debug_label = label;
        self
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Gif<'_>
where
    Renderer: image::Renderer<Handle = Handle>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State {
            id: self.frames.id,
            index: 0,
            current: self.frames.first.clone().into(),
        })
    }

    fn diff(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State>();
        // A reused tree slot now holding a different GIF: reset it to that GIF's
        // first frame. Keyed on the content hash, so distinct same-size emotes
        // are correctly distinguished (the upstream `total_bytes` proxy did not).
        if state.id != self.frames.id {
            tracing::warn!(
                label = self.debug_label,
                old_id = state.id,
                new_id = self.frames.id,
                "animated_image: widget slot reused for a different gif (diff) — resetting to the new gif's first frame"
            );
            *state = State {
                id: self.frames.id,
                index: 0,
                current: self.frames.first.clone().into(),
            };
        }
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        iced::widget::image::layout(
            renderer,
            limits,
            &self.frames.first.handle,
            self.width,
            self.height,
            None,
            self.content_fit,
            self.rotation,
            false,
        )
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();

        // Belt-and-suspenders alongside `diff`: if this tree slot was reused for
        // a different GIF, make sure the cached state belongs to *this* one
        // before advancing it — never animate the previous occupant's frames.
        if state.id != self.frames.id {
            tracing::warn!(
                label = self.debug_label,
                old_id = state.id,
                new_id = self.frames.id,
                "animated_image: widget slot reused for a different gif (on_event) — resetting"
            );
            *state = State {
                id: self.frames.id,
                index: 0,
                current: self.frames.first.clone().into(),
            };
        }

        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            let now = *now;
            let elapsed = now.duration_since(state.current.started);

            if elapsed > state.current.frame.delay {
                state.index = (state.index + 1) % self.frames.frames.len();
                state.current = self.frames.frames[state.index].clone().into();
                shell.request_redraw_at(now + state.current.frame.delay);
            } else {
                let remaining = state.current.frame.delay - elapsed;
                shell.request_redraw_at(now + remaining);
            }
        }
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
        let state = tree.state.downcast_ref::<State>();
        // Draw THIS gif's frame. If the slot was reused for a different gif and
        // no diff/on_event pass has reset `state` yet, `state.current` still
        // holds the previous occupant's frame — fall back to our own first frame
        // so a distinct same-size gif is never shown as a copy of another. (draw
        // takes `&Tree`, so it can't reset; on_event/diff do the real reset.)
        let handle = if state.id == self.frames.id {
            &state.current.frame.handle
        } else {
            // If this ever fires, `diff`/`on_event` both missed a slot reuse
            // that `draw` alone caught — meaning the bug's mechanism differs
            // from the one this module was built to fix. Worth flagging loud.
            tracing::error!(
                label = self.debug_label,
                old_id = state.id,
                new_id = self.frames.id,
                "animated_image: draw() caught a stale slot that diff/on_event missed — using this gif's first frame instead of the stale one"
            );
            &self.frames.first.handle
        };
        iced::widget::image::draw(
            renderer,
            layout,
            handle,
            None,
            iced::border::Radius::default(),
            self.content_fit,
            self.filter_method,
            self.rotation,
            self.opacity,
            1.0,
        );
    }
}

impl<'a, Message, Theme, Renderer> From<Gif<'a>> for Element<'a, Message, Theme, Renderer>
where
    Renderer: image::Renderer<Handle = Handle> + 'a,
{
    fn from(gif: Gif<'a>) -> Self {
        Element::new(gif)
    }
}
