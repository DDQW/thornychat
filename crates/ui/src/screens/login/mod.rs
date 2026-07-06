//! Login screen: a two-step wizard mirroring how Element decides what to
//! show. Step 1 asks only for a homeserver and discovers its supported
//! login flows (`GET /_matrix/client/v3/login`); step 2 shows a password
//! form, one button per SSO identity provider, or both — whatever the
//! homeserver actually advertises. Many homeservers (e.g. ones delegating
//! auth to a forum account) only support `m.login.sso` and never show a
//! password field at all.

use client_core::session::LoginFlows;
use iced::widget::{button, column, container, row, text, text_input};
use iced::{Element, Length, Task};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Default)]
pub struct State {
    pub homeserver: String,
    pub username: String,
    pub password: String,
    pub step: Step,
    pub status: Status,
}

#[derive(Debug, Clone, Default)]
pub enum Step {
    #[default]
    EnterHomeserver,
    ChooseMethod(LoginFlows),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Idle,
    Discovering,
    LoggingIn,
    Error(String),
}

#[derive(Debug, Clone)]
pub enum Message {
    HomeserverChanged(String),
    UsernameChanged(String),
    PasswordChanged(String),
    ContinueFromHomeserver,
    Back,
    SubmitPassword,
    ProviderChosen(Option<String>),

    /// Fed back in by the root dispatcher once the async discovery/login
    /// calls resolve; not produced directly by the view.
    FlowsDiscovered(LoginFlows),
    DiscoverFailed(String),
    LoginFailed(String),
}

/// What the root `update()` should do in response to a login action. Kept
/// separate from `Message` so this module never needs to know about
/// `client_core::RunningClient`, tokio, or the root `Message` enum.
pub enum Effect {
    None,
    Discover { homeserver: String },
    AttemptPasswordLogin { homeserver: String, username: String, password: Zeroizing<String> },
    AttemptSsoLogin { homeserver: String, identity_provider_id: Option<String> },
}

pub fn update(state: &mut State, message: Message) -> (Task<Message>, Effect) {
    match message {
        Message::HomeserverChanged(v) => {
            state.homeserver = v;
            (Task::none(), Effect::None)
        }
        Message::UsernameChanged(v) => {
            state.username = v;
            (Task::none(), Effect::None)
        }
        Message::PasswordChanged(v) => {
            state.password = v;
            (Task::none(), Effect::None)
        }
        Message::ContinueFromHomeserver => {
            if state.homeserver.trim().is_empty() {
                state.status = Status::Error("Enter a homeserver to continue.".into());
                return (Task::none(), Effect::None);
            }
            state.status = Status::Discovering;
            let effect = Effect::Discover { homeserver: state.homeserver.trim().to_string() };
            (Task::none(), effect)
        }
        Message::Back => {
            state.step = Step::EnterHomeserver;
            state.status = Status::Idle;
            (Task::none(), Effect::None)
        }
        Message::FlowsDiscovered(flows) => {
            state.step = Step::ChooseMethod(flows);
            state.status = Status::Idle;
            (Task::none(), Effect::None)
        }
        Message::DiscoverFailed(reason) => {
            state.status = Status::Error(reason);
            (Task::none(), Effect::None)
        }
        Message::SubmitPassword => {
            if state.username.trim().is_empty() {
                state.status = Status::Error("Username is required.".into());
                return (Task::none(), Effect::None);
            }
            state.status = Status::LoggingIn;
            let effect = Effect::AttemptPasswordLogin {
                homeserver: state.homeserver.trim().to_string(),
                username: state.username.trim().to_string(),
                password: Zeroizing::new(std::mem::take(&mut state.password)),
            };
            (Task::none(), effect)
        }
        Message::ProviderChosen(identity_provider_id) => {
            state.status = Status::LoggingIn;
            let effect = Effect::AttemptSsoLogin {
                homeserver: state.homeserver.trim().to_string(),
                identity_provider_id,
            };
            (Task::none(), effect)
        }
        Message::LoginFailed(reason) => {
            state.status = Status::Error(reason);
            (Task::none(), Effect::None)
        }
    }
}

pub fn view(state: &State) -> Element<'_, Message> {
    match &state.step {
        Step::EnterHomeserver => view_enter_homeserver(state),
        Step::ChooseMethod(flows) => view_choose_method(state, flows),
    }
}

fn status_text(status: &Status) -> Element<'_, Message> {
    match status {
        Status::Idle => text("").into(),
        Status::Discovering => text("Checking homeserver...").into(),
        Status::LoggingIn => text("Signing in...").into(),
        Status::Error(e) => text(e.clone()).style(text::danger).into(),
    }
}

fn view_enter_homeserver(state: &State) -> Element<'_, Message> {
    let form = column![
        text("ThornyChat").size(28),
        text("Enter your homeserver to sign in").size(14),
        text_input("Homeserver (e.g. matrix.org)", &state.homeserver)
            .on_input(Message::HomeserverChanged)
            .on_submit(Message::ContinueFromHomeserver)
            .padding(8),
        button(text("Continue")).on_press(Message::ContinueFromHomeserver).padding(8),
        status_text(&state.status),
    ]
    .spacing(12)
    .max_width(360);

    container(form)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
}

fn view_choose_method<'a>(state: &'a State, flows: &'a LoginFlows) -> Element<'a, Message> {
    let mut form = column![
        text("ThornyChat").size(28),
        text(state.homeserver.clone()).size(14),
    ]
    .spacing(12)
    .max_width(360);

    if flows.supports_password {
        form = form.push(
            column![
                text_input("Username", &state.username)
                    .on_input(Message::UsernameChanged)
                    .padding(8),
                text_input("Password", &state.password)
                    .on_input(Message::PasswordChanged)
                    .secure(true)
                    .on_submit(Message::SubmitPassword)
                    .padding(8),
                button(text("Sign in")).on_press(Message::SubmitPassword).padding(8),
            ]
            .spacing(8),
        );
    }

    if flows.supports_sso {
        if flows.sso_providers.is_empty() {
            form = form.push(
                button(text("Continue with SSO"))
                    .on_press(Message::ProviderChosen(None))
                    .padding(8),
            );
        } else {
            let mut providers = column![].spacing(8);
            for provider in &flows.sso_providers {
                providers = providers.push(
                    button(text(format!("Continue with {}", provider.name)))
                        .on_press(Message::ProviderChosen(Some(provider.id.clone())))
                        .padding(8),
                );
            }
            form = form.push(providers);
        }
    }

    form = form.push(row![
        button(text("Back")).on_press(Message::Back).padding(8),
    ]);
    form = form.push(status_text(&state.status));

    container(form)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
}
