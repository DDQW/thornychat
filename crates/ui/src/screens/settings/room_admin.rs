//! Power levels editor, member management, room settings, space
//! administration, room creation wizard. Phase 7.

use iced::widget::text;
use iced::Element;

pub fn view<'a, M: 'a>() -> Element<'a, M> {
    text("Room admin tools are coming in a later phase.").size(13).into()
}
