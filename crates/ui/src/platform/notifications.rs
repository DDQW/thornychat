//! Native WinRT toast notifications, raised through the AUMID registered by
//! `platform::app_identity`. Without that registration `show` returns `Ok`
//! and nothing appears on screen — the notification platform silently drops
//! toasts from an identity it can't resolve — so treat "no toast, no error"
//! as "run `cargo xtask install-dev`", not as a bug here.
//!
//! Plain title + body for now. Action buttons and inline reply need the COM
//! activation callback behind `app_identity::TOAST_ACTIVATOR_CLSID`, which is
//! registered but not yet implemented; the payload below is already
//! `ToastGeneric`, so those grow as extra XML rather than a rewrite.

use windows::core::HSTRING;
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

use super::app_identity::AUMID;

#[derive(Debug, Clone)]
pub enum Message {}

/// Shows a toast. Call from the UI thread: WinRT activation borrows the
/// calling thread's COM apartment, which iced has already initialized there.
pub fn show(title: &str, body: &str) -> windows::core::Result<()> {
    let payload = format!(
        "<toast>\
           <visual>\
             <binding template=\"ToastGeneric\">\
               <text>{}</text>\
               <text>{}</text>\
             </binding>\
           </visual>\
         </toast>",
        escape(title),
        escape(body),
    );

    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(payload))?;

    let toast = ToastNotification::CreateToastNotification(&document)?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?.Show(&toast)
}

/// The payload is XML, and message bodies are arbitrary user text — an
/// unescaped `<` or `&` from a peer would fail `LoadXml` (or worse, inject
/// markup into the toast). Only these three matter inside a text node.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::escape;

    #[test]
    fn escapes_markup_significant_characters() {
        assert_eq!(escape("a & b <c> d"), "a &amp; b &lt;c&gt; d");
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        assert_eq!(escape("hey — did you see this? 🌹"), "hey — did you see this? 🌹");
    }
}
