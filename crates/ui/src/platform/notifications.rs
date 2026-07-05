//! Native WinRT toast notifications with inline reply, driven off
//! `ClientEvent::Notification`. Requires package identity (MSIX) or a
//! registered AUMID to get action buttons — validate early per the plan's
//! risk notes. Phase 7.

#[derive(Debug, Clone)]
pub enum Message {}
