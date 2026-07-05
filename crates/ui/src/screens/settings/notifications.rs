//! Push-rule-backed notification settings: mute/highlight/per-room
//! override. Phase 5.

use iced::widget::text;
use iced::Element;

pub fn view<'a, M: 'a>() -> Element<'a, M> {
    text("Notification settings are coming in a later phase.").size(13).into()
}
