//! Custom emoji in outgoing messages, sent the way other clients send and
//! draw them (MSC2545): a `:shortcode:` naming a pack emoji becomes
//! `<img data-mx-emoticon src="mxc://…" alt=":shortcode:" …>` in the
//! `formatted_body`, while the plain `body` keeps the `:shortcode:` text for
//! clients that can't show images (IRC). Sent as bare text, which is what this
//! client used to do, everyone else saw the shortcode instead of the picture.
//!
//! The rewrite runs over the HTML that Markdown produced, so code spans,
//! code blocks and link text keep their colons.
//!
//! Going the other way, [`in_html`] reads the custom emoji out of someone's
//! message, for the places that only show its plain body (reply quotes).

use std::collections::HashMap;

use matrix_sdk::ruma::events::room::message::{FormattedBody, MessageFormat};
use matrix_sdk::ruma::html::{Html, NodeRef};

use crate::events::{CustomEmoji, InlineEmoticon};

/// The `height` other clients get (Cinny's value; they draw it text-sized).
const HEIGHT: u32 = 32;

/// Longest shortcode recognised, as in the frontend's matcher.
const MAX_SHORTCODE_LEN: usize = 64;

/// Gives a message's `formatted` body an inline image for every `:shortcode:`
/// in it that names one of `emoticons` (case-insensitively). A body Markdown
/// left unformatted is escaped into HTML first. Nothing changes when no
/// shortcode matched.
pub(crate) fn apply(body: &str, formatted: &mut Option<FormattedBody>, emoticons: &[CustomEmoji]) {
    let by_code: HashMap<String, &CustomEmoji> = emoticons
        .iter()
        .filter(|emoji| emoji.mxc_url.starts_with("mxc://") && is_shortcode(&emoji.shortcode))
        .map(|emoji| (emoji.shortcode.to_ascii_lowercase(), emoji))
        .collect();
    if by_code.is_empty() {
        return;
    }
    let html = match formatted {
        Some(existing) if existing.format == MessageFormat::Html => existing.body.clone(),
        Some(_) => return,
        None => plain_to_html(body),
    };
    if let Some(rewritten) = replace_shortcodes(&html, &by_code) {
        *formatted = Some(FormattedBody::html(rewritten));
    }
}

fn is_shortcode(code: &str) -> bool {
    (1..=MAX_SHORTCODE_LEN).contains(&code.len())
        && code.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-'))
}

fn escape_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
}

/// A plain body as HTML: escaped, line breaks kept.
fn plain_to_html(body: &str) -> String {
    let mut out = String::with_capacity(body.len() + 16);
    for (index, line) in body.split('\n').enumerate() {
        if index > 0 {
            out.push_str("<br />");
        }
        escape_into(&mut out, line.strip_suffix('\r').unwrap_or(line));
    }
    out
}

/// `html` with the matching shortcodes in its text replaced, or `None` if
/// there were none. Text inside `<code>`, `<pre>` and `<a>` is left alone.
fn replace_shortcodes(html: &str, emoticons: &HashMap<String, &CustomEmoji>) -> Option<String> {
    let mut out = String::with_capacity(html.len() + 128);
    let mut changed = false;
    let mut verbatim_depth = 0usize;
    let mut rest = html;
    while !rest.is_empty() {
        let end = if rest.starts_with("<!--") {
            rest.find("-->").map_or(rest.len(), |index| index + 3)
        } else if rest.starts_with('<') {
            let end = tag_end(rest);
            if let Some((name, closing, self_closing)) = tag_name(&rest[..end]) {
                if ["code", "pre", "a"].iter().any(|verbatim| name.eq_ignore_ascii_case(verbatim)) && !self_closing {
                    verbatim_depth = if closing { verbatim_depth.saturating_sub(1) } else { verbatim_depth + 1 };
                }
            }
            end
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            if verbatim_depth == 0 {
                changed |= replace_in_text(&rest[..end], emoticons, &mut out);
                rest = &rest[end..];
                continue;
            }
            end
        };
        out.push_str(&rest[..end]);
        rest = &rest[end..];
    }
    changed.then_some(out)
}

/// Byte length of the tag `html` starts with, through its `>` (quoted
/// attribute values may contain one).
fn tag_end(html: &str) -> usize {
    let mut quote = None;
    for (index, byte) in html.bytes().enumerate().skip(1) {
        match (quote, byte) {
            (Some(open), _) if byte == open => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(byte),
            (None, b'>') => return index + 1,
            (None, _) => {}
        }
    }
    html.len()
}

/// `(name, is_closing, is_self_closing)` of a tag like `<a href="…">`,
/// `</a>` or `<br />`.
fn tag_name(tag: &str) -> Option<(&str, bool, bool)> {
    let inner = tag.strip_prefix('<')?;
    let (closing, inner) = match inner.strip_prefix('/') {
        Some(after) => (true, after),
        None => (false, inner),
    };
    let len = inner.bytes().take_while(u8::is_ascii_alphanumeric).count();
    (len > 0).then(|| (&inner[..len], closing, tag.trim_end_matches('>').ends_with('/')))
}

/// Appends `text` (already-escaped HTML text) to `out` with every matching
/// `:shortcode:` swapped for its image; returns whether any was.
fn replace_in_text(text: &str, emoticons: &HashMap<String, &CustomEmoji>, out: &mut String) -> bool {
    let mut changed = false;
    let mut plain_from = 0;
    let mut search_from = 0;
    while let Some(open) = text[search_from..].find(':').map(|index| search_from + index) {
        let Some(close) = text[open + 1..].find(':').map(|index| open + 1 + index) else { break };
        let code = &text[open + 1..close];
        let emoji = is_shortcode(code).then(|| emoticons.get(&code.to_ascii_lowercase())).flatten();
        let Some(emoji) = emoji else {
            // The closing colon may open the next one ("12:30:wave:").
            search_from = close;
            continue;
        };
        out.push_str(&text[plain_from..open]);
        push_image(out, emoji);
        changed = true;
        plain_from = close + 1;
        search_from = close + 1;
    }
    out.push_str(&text[plain_from..]);
    changed
}

fn push_image(out: &mut String, emoji: &CustomEmoji) {
    out.push_str("<img data-mx-emoticon src=\"");
    escape_into(out, &emoji.mxc_url);
    out.push_str("\" alt=\":");
    escape_into(out, &emoji.shortcode);
    out.push_str(":\" title=\":");
    escape_into(out, &emoji.shortcode);
    out.push_str(&format!(":\" height=\"{HEIGHT}\" />"));
}

/// Most emoticons [`in_html`] returns: a quote shows one short line.
const MAX_FOUND: usize = 32;

/// The custom emoji an already-sanitized `formatted_body` draws, first of each
/// name. That is every `<img src="mxc://…">` with a usable alt text: by now
/// matrix-sdk has dropped `data-mx-emoticon`, and an image inside message
/// HTML is a custom emoji in practice (a picture is its own `m.image`). Cinny
/// writes the alt as `name`, this client as `:name:`; the plain body says
/// `:name:` either way.
pub(crate) fn in_html(html: &str) -> Vec<InlineEmoticon> {
    let mut found = Vec::new();
    for node in Html::parse(html).children() {
        collect_images(&node, &mut found);
    }
    found
}

fn collect_images(node: &NodeRef, found: &mut Vec<InlineEmoticon>) {
    if found.len() >= MAX_FOUND {
        return;
    }
    if let Some(element) = node.as_element().filter(|element| &*element.name.local == "img") {
        let attrs = element.attrs.borrow();
        let attr = |name: &str| attrs.iter().find(|attr| &*attr.name.local == name).map(|attr| attr.value.to_string());
        if let (Some(src), Some(alt)) = (attr("src"), attr("alt")) {
            let shortcode = alt.trim().trim_matches(':');
            if src.starts_with("mxc://") && is_shortcode(shortcode) && !found.iter().any(|seen| seen.shortcode == shortcode) {
                found.push(InlineEmoticon { shortcode: shortcode.to_string(), mxc_url: src });
            }
        }
    }
    for child in node.children() {
        collect_images(&child, found);
    }
}

#[cfg(test)]
mod tests {
    use matrix_sdk::ruma::events::room::message::TextMessageEventContent;

    use super::*;

    fn emoji(shortcode: &str, mxc_url: &str) -> CustomEmoji {
        CustomEmoji {
            shortcode: shortcode.into(),
            mxc_url: mxc_url.into(),
            is_emoticon: true,
            is_sticker: false,
            width: None,
            height: None,
        }
    }

    /// What `SendMessage` does with a Markdown body.
    fn send(body: &str, emoticons: &[CustomEmoji]) -> TextMessageEventContent {
        let mut text = TextMessageEventContent::markdown(body);
        apply(&text.body, &mut text.formatted, emoticons);
        text
    }

    fn html(content: &TextMessageEventContent) -> Option<&str> {
        content.formatted.as_ref().map(|formatted| formatted.body.as_str())
    }

    #[test]
    fn a_plain_message_gets_html_with_the_image() {
        let sent = send("we could all play a mud :scratch:", &[emoji("scratch", "mxc://hq.example/abc")]);
        assert_eq!(sent.body, "we could all play a mud :scratch:");
        assert_eq!(
            html(&sent),
            Some(r#"we could all play a mud <img data-mx-emoticon src="mxc://hq.example/abc" alt=":scratch:" title=":scratch:" height="32" />"#)
        );
    }

    #[test]
    fn shortcodes_match_case_insensitively_and_keep_the_pack_spelling() {
        let sent = send(":notsure:", &[emoji("NotSure", "mxc://hq.example/ns")]);
        assert!(html(&sent).unwrap().contains(r#"alt=":NotSure:""#), "{:?}", html(&sent));
    }

    #[test]
    fn markdown_formatting_survives() {
        let sent = send("**loud** :scratch:\nsecond line", &[emoji("scratch", "mxc://hq.example/abc")]);
        let out = html(&sent).unwrap();
        assert!(out.contains("<strong>loud</strong> <img data-mx-emoticon"), "{out}");
        assert!(out.contains("second line"), "{out}");
    }

    #[test]
    fn plain_line_breaks_and_markup_characters_are_kept_safe() {
        let sent = send("a < b :scratch:\nnext", &[emoji("scratch", "mxc://hq.example/abc")]);
        let out = html(&sent).unwrap();
        assert!(out.starts_with("a &lt; b <img"), "{out}");
        assert!(out.contains("<br />next") || out.contains("<br>\nnext") || out.contains("<br />\nnext"), "{out}");
    }

    #[test]
    fn code_and_links_keep_their_text() {
        let codes = [emoji("scratch", "mxc://hq.example/abc")];
        let inline = send("type `:scratch:` for it", &codes);
        assert!(html(&inline).is_none_or(|out| !out.contains("<img")), "{:?}", html(&inline));
        let block = send("```\n:scratch:\n```\n:scratch:", &codes);
        let out = html(&block).unwrap();
        assert_eq!(out.matches("<img").count(), 1, "{out}");
        assert!(out.contains("<code>:scratch:\n</code>"), "{out}");
        let link = send("[:scratch:](https://example.org/a:b:c)", &codes);
        assert!(html(&link).is_none_or(|out| !out.contains("<img")), "{:?}", html(&link));
    }

    #[test]
    fn unknown_shortcodes_and_times_stay_text() {
        let codes = [emoji("wave", "mxc://hq.example/w")];
        let sent = send("meet at 12:30:wave: or :nope:", &codes);
        let out = html(&sent).unwrap();
        assert!(out.starts_with("meet at 12:30<img"), "{out}");
        assert!(out.ends_with(" or :nope:"), "{out}");
        let untouched = send("12:30 and :nope:", &codes);
        assert!(html(&untouched).is_none());
    }

    #[test]
    fn nothing_changes_without_emoticons() {
        let sent = send("plain :scratch:", &[]);
        assert!(html(&sent).is_none());
    }

    #[test]
    fn attribute_values_are_escaped() {
        let sent = send(":x:", &[emoji("x", "mxc://hq.example/a\"onerror=\"b")]);
        let out = html(&sent).unwrap();
        assert!(out.contains(r#"src="mxc://hq.example/a&quot;onerror=&quot;b""#), "{out}");
    }

    #[test]
    fn tags_with_a_quoted_gt_are_skipped_whole() {
        let codes: HashMap<String, &CustomEmoji> = HashMap::new();
        assert_eq!(tag_end(r#"<a title="x>y">rest"#), r#"<a title="x>y">"#.len());
        assert_eq!(tag_name("</code>"), Some(("code", true, false)));
        assert_eq!(tag_name("<br />"), Some(("br", false, true)));
        assert!(replace_shortcodes("<p>:a:</p>", &codes).is_none());
    }

    #[test]
    fn emoticons_are_read_back_out_of_html() {
        // Cinny's shape after matrix-sdk's sanitizer, then this client's own.
        let html = concat!(
            r#"<p>Doing what? <img src="mxc://rpghq.org/ZHrq" alt="NotSure" title="NotSure" height="32" />"#,
            r#" <b>and <img src="mxc://rpghq.org/mhm" alt=":mhm:" height="32" /></b>"#,
            r#" again <img src="mxc://rpghq.org/other" alt="NotSure" /></p>"#,
        );
        let found = in_html(html);
        assert_eq!(
            found,
            vec![
                InlineEmoticon { shortcode: "NotSure".into(), mxc_url: "mxc://rpghq.org/ZHrq".into() },
                InlineEmoticon { shortcode: "mhm".into(), mxc_url: "mxc://rpghq.org/mhm".into() },
            ]
        );
    }

    #[test]
    fn images_that_are_not_emoticons_are_skipped() {
        let html = concat!(
            r#"<img src="mxc://hq.example/noalt" />"#,
            r#"<img src="mxc://hq.example/spaces" alt="two words" />"#,
            r#"<img src="https://hq.example/remote.png" alt="remote" />"#,
        );
        assert!(in_html(html).is_empty());
        assert!(in_html("no images at all").is_empty());
    }
}
