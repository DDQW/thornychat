//! Security banners: cross-signing bootstrap fallback and the SAS device
//! verification wizard, rendered as a stack of cards above the room list
//! rather than a modal overlay — simpler, and none of these need to block
//! the rest of the app. Key backup/recovery and starting a new verification
//! live in Settings → Security (`recovery_settings_view`) instead.

use client_core::events::{RecoveryEnableStage, SasState};
use iced::widget::{button, column, container, row, text, text_input};
use iced::{Element, Length};

#[derive(Debug, Clone, Default)]
pub struct State {
    pub cross_signing_fallback_url: Option<String>,
    pub cross_signing_error: Option<String>,

    pub recovery_setup_needed: bool,
    /// Set when matrix-sdk reports this device is missing room keys. No longer
    /// used to drive a prompt — the Security tab always exposes a recovery-key
    /// field rather than nagging — but kept for possible status display.
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
}

#[derive(Debug, Clone)]
pub enum Message {
    OpenCrossSigningFallback,
    RetryCrossSigningBootstrap,
    DismissCrossSigningError,

    TogglePassphraseInput,
    PassphraseChanged(String),
    ConfirmEnableRecovery,
    RecoveryKeySaved,
    RecoveryKeyInputChanged(String),
    SubmitRecoveryKey,
    DismissRecoveryError,

    VerifyUserIdChanged(String),
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

        Message::TogglePassphraseInput => state.show_passphrase_input = !state.show_passphrase_input,
        Message::PassphraseChanged(v) => state.passphrase_input = v,
        Message::ConfirmEnableRecovery => {
            // In-flight guard: a double-click would run two concurrent
            // recovery bootstraps — the second regenerates the key, so the
            // first key shown to the user would already be dead. The stage
            // is set synchronously; RecoveryEnabled/RecoveryEnableFailed
            // clear it.
            if state.recovery_enable_stage.is_some() {
                return Effect::None;
            }
            state.recovery_enable_stage = Some(RecoveryEnableStage::Starting);
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
        Message::StartVerification => {
            // Blank is a real input here — the placeholder promises
            // "blank = verify this device"; the dispatcher maps an empty id
            // to the account's own user id (self-verification).
            let user_id = state.verify_user_id_input.trim().to_string();
            return Effect::StartVerification { user_id };
        }
        Message::AcceptVerification => return Effect::AcceptVerificationRequest,
        Message::DeclineVerification => {
            // Remove the card too — the flow is over on our side, and the
            // request card's buttons would otherwise sit dead forever (the
            // finished session ignores further actions).
            state.sas = None;
            return Effect::VerificationCancel;
        }
        Message::ConfirmEmojisMatch => return Effect::ConfirmSasMatch,
        Message::RejectEmojisMatch => {
            // Mismatch ends the flow; show the Cancelled card (which has a
            // Dismiss button) instead of leaving the emoji card stuck with
            // dead buttons.
            state.sas = Some(client_core::events::SasState::Cancelled {
                reason: "you indicated the emojis did not match".into(),
            });
            return Effect::RejectSasMatch;
        }
        Message::DismissSas => {
            state.sas = None;
            return Effect::VerificationCancel;
        }
    }
    Effect::None
}

pub fn view(state: &State) -> Element<'_, Message> {
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

    // Encryption backup/recovery no longer nags from here — it lives in
    // Settings → Security (see `recovery_settings_view`). Only genuinely
    // interactive, time-sensitive flows (an incoming verification request,
    // cross-signing bootstrap) stay as banners below.

    if let Some(sas) = &state.sas {
        cards = cards.push(sas_card(sas));
    }

    cards.into()
}

/// The encryption backup & recovery UI, in Settings → Security. ThornyChat never
/// prompts for a recovery key on its own: this tab just offers a calm,
/// always-available key field that does nothing until the user chooses to
/// enter a key (or, when no backup exists yet, to set one up). No launch-time
/// nagging — the "recovery needed" state is deliberately *not* used to push a
/// prompt at the user; they act here when they want to.
pub fn recovery_settings_view(state: &State) -> Element<'_, Message> {
    let mut cards = column![].spacing(8);

    // One-time reveal right after generating a fresh key: show it on its own
    // until the user confirms they've saved it (it can never be shown again).
    if let Some(key) = &state.recovery_key_to_confirm {
        cards = cards.push(card(column![
            text("Save your recovery key").size(14),
            text("This is the only time this key will be shown. Store it somewhere safe — you'll need it to read your encrypted message history on a new device.").size(12),
            container(text(key.clone()).size(16)).padding(8),
            button(text("I've saved it")).on_press(Message::RecoveryKeySaved).padding(6),
        ]));
        return cards.into();
    }

    // Steady state: a single opt-in section. The key field just sits here,
    // inert, until the user types a key and hits Unlock — not gated on any
    // "you need to do this" signal from the SDK, so the app never asks.
    let mut section = column![
        text("Encrypted message backup").size(14),
        text(
            "Optional — ThornyChat won't ask for this on its own. If you have a recovery key \
             or passphrase, enter it here to unlock your encrypted message history on this \
             device. Nothing is sent or changed until you do.",
        )
        .size(12)
        .style(text::secondary),
        text_input("Recovery key or passphrase", &state.recovery_key_input)
            .on_input(Message::RecoveryKeyInputChanged)
            .on_submit(Message::SubmitRecoveryKey)
            .secure(true)
            .padding(6),
        button(text("Unlock")).on_press(Message::SubmitRecoveryKey).padding(6),
    ]
    .spacing(6);

    // Offer creating a fresh backup only when the account has none yet, so we
    // never silently replace an existing recovery key (enabling again mints a
    // new key and invalidates the old one).
    if state.recovery_setup_needed {
        section = section.push(
            text("Don't have a recovery key? Set up a new backup so future messages stay recoverable on your other devices.")
                .size(12)
                .style(text::secondary),
        );

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

        // No .on_press while a bootstrap is in flight — iced renders it
        // disabled, mirroring the guard in the ConfirmEnableRecovery arm.
        let mut setup_button = button(text("Set up a new backup")).padding(6);
        if state.recovery_enable_stage.is_none() {
            setup_button = setup_button.on_press(Message::ConfirmEnableRecovery);
        }
        section = section.push(
            row![
                button(text(if state.show_passphrase_input { "No passphrase" } else { "Use a passphrase" }))
                    .on_press(Message::TogglePassphraseInput)
                    .padding(6),
                setup_button,
            ]
            .spacing(6),
        );
    }

    cards = cards.push(card(section));

    if let Some(error) = &state.recovery_error {
        cards = cards.push(card(column![
            text(error.clone()).size(12).style(text::danger),
            button(text("Dismiss")).on_press(Message::DismissRecoveryError).padding(6),
        ]));
    }

    // An incoming request shows up on its own (see `view`'s SAS card) — this
    // is only for starting one. Hidden while a SAS flow is already active so
    // there's never a second one in flight.
    if state.sas.is_none() {
        cards = cards.push(card(column![
            text("Device verification").size(14),
            text(
                "Verify another session or a contact so they show as trusted. Leave the \
                 field blank to verify this device instead.",
            )
            .size(12)
            .style(text::secondary),
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
