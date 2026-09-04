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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use iced::advanced::image::{self, FilterMethod, Handle};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{Clipboard, Shell, Widget};
use iced::{mouse, window, ContentFit, Element, Event, Length, Rectangle, Rotation, Size};

/// Ceiling on the total decoded pixels of a single animated image.
///
/// Frames are held as fully decoded RGBA and never evicted, so the real cost
/// is this times four bytes: 64M pixels = ~256 MB worst case for one pathological
/// GIF, and the ordinary emote (64x64, 20 frames = 82k pixels) is four orders of
/// magnitude under it. High enough that nothing anyone actually posts is
/// truncated; low enough that a decompression-bomb GIF cannot exhaust memory.
const MAX_DECODED_PIXELS: u64 = 64 * 1024 * 1024;

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

        // Bounded rather than `collect()`ed wholesale: every frame is kept as
        // fully decoded RGBA (see `Frame::from`), so cost is
        // width * height * 4 * frame_count with nothing in `media_cache` ever
        // evicting it. One 100-frame 512x512 GIF is ~105 MB resident, and a
        // pack of animated emotes is fetched and decoded in bulk. Past the
        // budget we stop decoding and keep what we have — the animation runs
        // short rather than the image failing, which is far better than either
        // dropping it or letting one message cost hundreds of megabytes.
        let mut frames: Vec<Frame> = Vec::new();
        let mut pixels: u64 = 0;
        for result in decoder.into_frames() {
            let frame = Frame::from(result.ok()?);
            pixels += u64::from(frame.width) * u64::from(frame.height);
            frames.push(frame);
            if pixels >= MAX_DECODED_PIXELS {
                tracing::debug!(
                    frames = frames.len(),
                    pixels,
                    "animated_image: gif hit the decode budget, truncating the animation"
                );
                break;
            }
        }
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

    /// Resident bytes of the decoded frames, for the cache's byte budget.
    ///
    /// `first` is a clone of `frames[0]` and an `image::Handle` shares its
    /// pixel buffer when cloned, so it is deliberately not counted twice.
    pub fn decoded_bytes(&self) -> u64 {
        self.frames
            .iter()
            .map(|f| u64::from(f.width) * u64::from(f.height) * 4)
            .sum()
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

/// Shared animation cadence, ~30 fps.
///
/// `Shell::request_redraw_at` keeps the *earliest* instant any widget asks for,
/// so GIFs left to their own schedules wake the window at the union of all of
/// them: six emotes with the same 100 ms frame delay but different start times
/// cost six whole-window redraws per 100 ms instead of one. Rounding every
/// deadline up onto one grid collapses everything due in the same tick into a
/// single frame, which is what "in sync" has to mean here — one window, one
/// redraw, all the emotes advanced together.
///
/// The cost is up to 33 ms of timing error per frame. Emote GIFs are almost
/// always 100 ms per frame (browsers clamp anything under 20 ms to 100 ms), so
/// that is not perceptible; a GIF asking for more than 30 fps is capped, which
/// is already true of most displays.
const ANIMATION_TICK: Duration = Duration::from_millis(33);

/// Fixed origin for the tick grid. Any instant works as long as every GIF
/// shares it — that shared phase is the whole point.
static ANIMATION_EPOCH: OnceLock<Instant> = OnceLock::new();

/// Whether the window currently has focus.
///
/// Process-wide rather than per-widget: it describes the window, and a GIF
/// scrolled into view while the app sits in the background has to start paused
/// too, having never seen the `Unfocused` event that put it there.
static WINDOW_FOCUSED: AtomicBool = AtomicBool::new(true);

fn set_window_focused(focused: bool) {
    WINDOW_FOCUSED.store(focused, Ordering::Relaxed);
}

fn window_focused() -> bool {
    WINDOW_FOCUSED.load(Ordering::Relaxed)
}

/// Rounds `target` up to the next point on the shared [`ANIMATION_TICK`] grid,
/// so GIFs due within the same tick ask for the same instant.
fn next_tick(target: Instant) -> Instant {
    let epoch = *ANIMATION_EPOCH.get_or_init(Instant::now);
    let elapsed = target.saturating_duration_since(epoch).as_millis();
    let tick = ANIMATION_TICK.as_millis();
    let ticks = elapsed.div_ceil(tick);
    // Saturating: a target absurdly far out would otherwise wrap the cast.
    let offset = u64::try_from(ticks.saturating_mul(tick)).unwrap_or(u64::MAX);
    epoch + Duration::from_millis(offset)
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

        match event {
            Event::Window(window::Event::Unfocused) => set_window_focused(false),
            Event::Window(window::Event::Focused) => {
                set_window_focused(true);
                // Restart this GIF's clock instead of letting it catch up: the
                // frames nobody saw aren't worth replaying, and without this a
                // long spell in the background makes every emote lurch forward
                // on the way back.
                state.current.started = Instant::now();
                shell.request_redraw();
            }
            Event::Window(window::Event::RedrawRequested(now)) => {
                // Nothing to animate for a window nobody is looking at, and
                // every frame costs a whole-window redraw plus a texture
                // upload. Not requesting a redraw here is what actually stops
                // the work — the window then only wakes for real events.
                if !window_focused() {
                    return;
                }

                let now = *now;
                let elapsed = now.duration_since(state.current.started);

                if elapsed > state.current.frame.delay {
                    state.index = (state.index + 1) % self.frames.frames.len();
                    state.current = self.frames.frames[state.index].clone().into();
                    shell.request_redraw_at(next_tick(now + state.current.frame.delay));
                } else {
                    let remaining = state.current.frame.delay - elapsed;
                    shell.request_redraw_at(next_tick(now + remaining));
                }
            }
            _ => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the epoch so the grid is deterministic across the test module.
    fn epoch() -> Instant {
        *ANIMATION_EPOCH.get_or_init(Instant::now)
    }

    #[test]
    fn deadlines_land_on_the_shared_grid() {
        let epoch = epoch();
        for ms in [0u64, 1, 32, 33, 34, 99, 100, 5_000] {
            let at = next_tick(epoch + Duration::from_millis(ms));
            let offset = at.saturating_duration_since(epoch).as_millis();
            assert_eq!(
                offset % ANIMATION_TICK.as_millis(),
                0,
                "{ms}ms landed off-grid at {offset}ms"
            );
            assert!(offset >= u128::from(ms), "{ms}ms was rounded down to {offset}ms");
        }
    }

    #[test]
    fn emotes_due_in_the_same_tick_collapse_to_one_redraw() {
        // The case that motivated this: several emotes with the same frame
        // delay but different start phases. Previously each asked for its own
        // instant and woke the window separately.
        let epoch = epoch();
        // Phases strictly inside one tick interval. (A deadline landing exactly
        // on a grid point belongs to that point, so 0 is its own bucket.)
        let requests: std::collections::HashSet<_> = [1u64, 5, 11, 19, 27, 33]
            .iter()
            .map(|phase| next_tick(epoch + Duration::from_millis(*phase)))
            .collect();
        assert_eq!(requests.len(), 1, "six emotes within one tick must share a frame");

        // And the win this is really about: six 100 ms emotes at scattered
        // phases used to wake the window six times per 100 ms.
        let per_100ms: std::collections::HashSet<_> = [0u64, 17, 34, 51, 68, 85]
            .iter()
            .map(|phase| next_tick(epoch + Duration::from_millis(phase + 100)))
            .collect();
        assert!(
            per_100ms.len() <= 4,
            "expected scattered emotes to collapse, got {} distinct redraws",
            per_100ms.len()
        );
    }

    #[test]
    fn emotes_in_different_ticks_still_get_their_own_frames() {
        // Coalescing must not starve a GIF that is genuinely due later.
        let epoch = epoch();
        let early = next_tick(epoch + Duration::from_millis(10));
        let late = next_tick(epoch + Duration::from_millis(40));
        assert!(late > early);
    }

    #[test]
    fn a_frame_is_never_scheduled_early() {
        // Rounding down would show a frame before its delay had elapsed,
        // speeding every GIF up slightly.
        let epoch = epoch();
        for ms in 0..200u64 {
            let target = epoch + Duration::from_millis(ms);
            assert!(next_tick(target) >= target, "{ms}ms was scheduled early");
        }
    }

    #[test]
    fn the_tick_bounds_how_late_a_frame_can_be() {
        let epoch = epoch();
        for ms in 0..200u64 {
            let target = epoch + Duration::from_millis(ms);
            let slip = next_tick(target).saturating_duration_since(target);
            assert!(slip < ANIMATION_TICK, "{ms}ms slipped {slip:?}, past one tick");
        }
    }

    #[test]
    fn focus_gates_animation() {
        // The flag the redraw path consults before scheduling anything.
        set_window_focused(false);
        assert!(!window_focused());
        set_window_focused(true);
        assert!(window_focused());
    }
}
