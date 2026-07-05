//! General app settings (profile, autostart toggle). Phase 7. Theme lives
//! in the Appearance tab (see `screens::settings::appearance`), not here.

use iced::widget::text;
use iced::Element;

pub fn view<'a, M: 'a>() -> Element<'a, M> {
    text("More general settings are coming in a later phase.").size(13).into()
}
