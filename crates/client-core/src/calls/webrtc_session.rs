//! Native media for a joined MatrixRTC call — the future half of Phase 5.
//! Signaling (`calls::CallManager`) is real today; media is not: current
//! MatrixRTC calls run through a LiveKit focus (SFU), so this needs a
//! LiveKit protocol client (JWT from the focus' service URL, websocket
//! signaling, RTC via the `webrtc` crate), not just a raw peer connection.
//! Audio first, video second, per the plan's risk-mitigation ordering.
