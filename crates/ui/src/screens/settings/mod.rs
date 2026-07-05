pub mod general;
pub mod notifications;
pub mod room_admin;

use iced::widget::{column, text};
use iced::Element;

#[derive(Debug, Clone, Default)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Message {}

pub fn view(_state: &State) -> Element<'_, Message> {
    column![text("Settings (coming in later phases)")].into()
}
