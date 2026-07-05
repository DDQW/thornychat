//! Security banners: cross-signing bootstrap fallback, SAS device
//! verification wizard, and key backup/recovery setup + restore. Rendered
//! as a stack of cards below the sync-status line rather than a modal
//! overlay — simpler, and none of these need to block the rest of the app.

use client_core::events::{RecoveryEnableStage, SasState};
use iced::widget::{button, column, container, row, text, text_input};
use iced::{Element, Length};

#[derive(Debug, Clone, Default)]
pub struct State {
    pub cross_signing_fallback_url: Option<String>,
    pub cross_signing_error: Option<String>,

    pub recovery_setup_needed: bool,
    pub recovery_needs_key: bool,
    pub show_passphrase_input: bool,
    pub passphrase_input: String,
    pub recovery_enable_stage: Option<RecoveryEnableStage>,
    /// Shown exactly once after generating a fresh recovery key; cleared
    /// only once the user confirms they've saved it.
    pub recovery_key_to_confirm: Option<String>,
    pub recovery_key_input: String,
    pub recovery_error: Option<String>,

    pub sas: Option<SasState>,
    pub verify_user_id_input: String,
    /// The verify form is collapsed behind a small button by default —
    /// verification is an occasional task and the always-open form ate a
    /// full-width bar of every session.
    pub show_verify_form: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    OpenCrossSigningFallback,
    RetryCrossSigningBootstrap,
    DismissCrossSigningError,

    DismissRecoveryNeeded,
    TogglePassphraseInput,
    PassphraseChanged(String),
    ConfirmEnableRecovery,
    RecoveryKeySaved,
    RecoveryKeyInputChanged(String),
    SubmitRecoveryKey,
    DismissRecoveryError,

    VerifyUserIdChanged(String),
    ToggleVerifyForm,
    StartVerification,
    AcceptVerification,
    DeclineVerification,
    ConfirmEmojisMatch,
    RejectEmojisMatch,
    DismissSas,
}

pub enum Effect {
    None,
    OpenUrl(String),
    RetryCrossSigningBootstrap,
    EnableRecovery { passphrase: Option<String> },
    RestoreFromBackup { recovery_key: String },
    StartVerification { user_id: String },
    AcceptVerificationRequest,
    ConfirmSasMatch,
    RejectSasMatch,
    VerificationCancel,
}

pub fn update(state: &mut State, message: Message) -> Effect {
    match message {
        Message::OpenCrossSigningFallback => {
            if let Some(url) = state.cross_signing_fallback_url.clone() {
                return Effect::OpenUrl(url);
            }
        }
        Message::RetryCrossSigningBootstrap => return Effect::RetryCrossSigningBootstrap,
        Message::DismissCrossSigningError => state.cross_signing_error = None,

        Message::DismissRecoveryNeeded => state.recovery_needs_key = false,
        Message::TogglePassphraseInput => state.show_passphrase_input = !state.show_passphrase_input,
        Message::PassphraseChanged(v) => state.passphrase_input = v,
        Message::ConfirmEnableRecovery => {
            let passphrase = if state.show_passphrase_input && !state.passphrase_input.trim().is_empty() {
                Some(state.passphrase_input.trim().to_string())
            } else {
                None
            };
            return Effect::EnableRecovery { passphrase };
        }
        Message::RecoveryKeySaved => {
            state.recovery_key_to_confirm = None;
            state.passphrase_input.clear();
        }
        Message::RecoveryKeyInputChanged(v) => state.recovery_key_input = v,
        Message::SubmitRecoveryKey => {
            let recovery_key = state.recovery_key_input.trim().to_string();
            if recovery_key.is_empty() {
                return Effect::None;
            }
            return Effect::RestoreFromBackup { recovery_key };
        }
        Message::DismissRecoveryError => state.recovery_error = None,

        Message::VerifyUserIdChanged(v) => state.verify_user_id_input = v,
        Message::ToggleVerifyForm => state.show_verify_form = !state.show_verify_form,
        Message::StartVerification => {
            let user_id = state.verify_user_id_input.trim().to_string();
            if user_id.is_empty() {
                return Effect::None;
            }
            return Effect::StartVerification { user_id };
        }
        Message::AcceptVerification => return Effect::AcceptVerificationRequest,
        Message::DeclineVerification => return Effect::VerificationCancel,
        Message::ConfirmEmojisMatch => return Effect::ConfirmSasMatch,
        Message::RejectEmojisMatch => return Effect::RejectSasMatch,
        Message::DismissSas => {
            state.sas = None;
            return Effect::VerificationCancel;
        }
    }
    Effect::None
}

pub fn view<'a>(state: &'a State, own_user_id: Option<&'a str>) -> Element<'a, Message> {
    let mut cards = column![].spacing(8);

    if let Some(error) = &state.cross_signing_error {
        cards = cards.push(card(column![
            text("Couldn't set up secure messaging").size(14),
            text(error.clone()).size(12).style(text::danger),
            button(text("Dismiss")).on_press(Message::DismissCrossSigningError).padding(6),
        ]));
    }

    if state.cross_signing_fallback_url.is_some() {
        cards = cards.push(card(column![
            text("Finish setting up secure messaging").size(14),
            text("Your homeserver needs you to confirm this in your browser.").size(12),
            row![
                button(text("Open browser")).on_press(Message::OpenCrossSigningFallback).padding(6),
                button(text("I've done that, continue"))
                    .on_press(Message::RetryCrossSigningBootstrap)
                    .padding(6),
            ]
            .spacing(6),
        ]));
    }

    if let Some(key) = &state.recovery_key_to_confirm {
        cards = cards.push(card(column![
            text("Save your recovery key").size(14),
            text("This is the only time this key will be shown. Store it somewhere safe — you'll need it to read your encrypted message history on a new device.").size(12),
            container(text(key.clone()).size(16)).padding(8),
            button(text("I've saved it")).on_press(Message::RecoveryKeySaved).padding(6),
        ]));
    }

    if state.recovery_setup_needed && state.recovery_key_to_confirm.is_none() {
        let mut section = column![
            text("Set up secure message backup").size(14),
            text("Protects your encrypted messages so you can recover them on a new device.").size(12),
        ]
        .spacing(6);

        if state.show_passphrase_input {
            section = section.push(
                text_input("Optional passphrase", &state.passphrase_input)
                    .on_input(Message::PassphraseChanged)
                    .secure(true)
                    .padding(6),
            );
        }

        if let Some(stage) = state.recovery_enable_stage {
            section = section.push(text(stage_label(stage)).size(12));
        }

        section = section.push(
            row![
                button(text(if state.show_passphrase_input { "No passphrase" } else { "Use a passphrase" }))
                    .on_press(Message::TogglePassphraseInput)
                    .padding(6),
                button(text("Enable")).on_press(Message::ConfirmEnableRecovery).padding(6),
            ]
            .spacing(6),
        );

        cards = cards.push(card(section));
    }

    if state.recovery_needs_key {
        cards = cards.push(card(column![
            text("Unlock your encrypted history").size(14),
            text("This device is missing some encryption keys. Enter your recovery key or passphrase to unlock past messages.").size(12),
            text_input("Recovery key", &state.recovery_key_input)
                .on_input(Message::RecoveryKeyInputChanged)
                .on_submit(Message::SubmitRecoveryKey)
                .secure(true)
                .padding(6),
            row![
                button(text("Unlock")).on_press(Message::SubmitRecoveryKey).padding(6),
                button(text("Not now")).on_press(Message::DismissRecoveryNeeded).padding(6),
            ]
            .spacing(6),
        ]));
    }

    if let Some(error) = &state.recovery_error {
        cards = cards.push(card(column![
            text(error.clone()).size(12).style(text::danger),
            button(text("Dismiss")).on_press(Message::DismissRecoveryError).padding(6),
        ]));
    }

    if let Some(sas) = &state.sas {
        cards = cards.push(sas_card(sas));
    }

    // Collapsed to a small toggle unless the user opened the form or a SAS
    // flow is active (in which case the wizard card above already shows).
    if own_user_id.is_some() && state.show_verify_form && state.sas.is_none() {
        cards = cards.push(card(column![
            row![
                text("Device / user verification").size(14).width(Length::Fill),
                button(text("Hide").size(12))
                    .on_press(Message::ToggleVerifyForm)
                    .style(crate::theme::ghost_button)
                    .padding(4),
            ]
            .align_y(iced::Center),
            row![
                text_input("@user:server (blank = verify this device)", &state.verify_user_id_input)
                    .on_input(Message::VerifyUserIdChanged)
                    .padding(6)
                    .width(Length::Fill),
                button(text("Verify")).on_press(Message::StartVerification).padding(6),
            ]
            .spacing(6),
        ]));
    }

    cards.into()
}

/// Small toggle for the collapsed verify form, meant for the app's top
/// status bar (rendered there by `view.rs` so it doesn't occupy a card row
/// of its own).
pub fn verify_toggle(state: &State) -> Option<Element<'_, Message>> {
    if state.show_verify_form || state.sas.is_some() {
        return None;
    }
    Some(
        button(text("Verify device...").size(12))
            .on_press(Message::ToggleVerifyForm)
            .style(crate::theme::ghost_button)
            .padding([4, 8])
            .into(),
    )
}

fn sas_card(sas: &SasState) -> Element<'_, Message> {
    match sas {
        SasState::RequestReceived { from_user_id } => card(column![
            text(format!("{from_user_id} wants to verify")).size(14),
            row![
                button(text("Accept")).on_press(Message::AcceptVerification).padding(6),
                button(text("Decline")).on_press(Message::DeclineVerification).padding(6),
            ]
            .spacing(6),
        ]),
        SasState::RequestSent => card(column![
            text("Waiting for the other device...").size(14),
            button(text("Cancel")).on_press(Message::DismissSas).padding(6),
        ]),
        SasState::Ready => card(text("Starting verification...").size(14)),
        SasState::EmojisReady(emojis) => {
            let mut grid = row![].spacing(12);
            for (symbol, name) in emojis {
                grid = grid.push(column![text(symbol.clone()).size(28), text(name.clone()).size(11)].spacing(2));
            }
            card(column![
                text("Do these emoji match on both devices?").size(14),
                grid,
                row![
                    button(text("They match")).on_press(Message::ConfirmEmojisMatch).padding(6),
                    button(text("They don't match")).on_press(Message::RejectEmojisMatch).padding(6),
                ]
                .spacing(6),
            ])
        }
        SasState::WaitingForOtherPartyConfirmation => {
            card(text("Waiting for the other device to confirm...").size(14))
        }
        SasState::Done => card(column![
            text("Verification complete").size(14),
            button(text("Dismiss")).on_press(Message::DismissSas).padding(6),
        ]),
        SasState::Cancelled { reason } => card(column![
            text(format!("Verification cancelled: {reason}")).size(14),
            button(text("Dismiss")).on_press(Message::DismissSas).padding(6),
        ]),
    }
}

fn stage_label(stage: RecoveryEnableStage) -> &'static str {
    match stage {
        RecoveryEnableStage::Starting => "Starting...",
        RecoveryEnableStage::CreatingBackup => "Creating backup...",
        RecoveryEnableStage::CreatingRecoveryKey => "Creating recovery key...",
        RecoveryEnableStage::BackingUp => "Backing up room keys...",
    }
}

fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content).padding(10).width(Length::Fill).into()
}
