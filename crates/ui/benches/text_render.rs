//! Benchmarks for the per-message text scan — the loop `render_text_body`
//! runs for every text item on every view rebuild, plus the `unicode_emojis_in`
//! pass `update.rs` runs over every body when a timeline diff lands.
//!
//! The corpora below are shaped like real chat traffic rather than synthetic
//! worst cases: mostly short ASCII lines, a good share of accented Latin
//! (this is a German-speaking user's client), the odd link, and emoji used
//! the way people actually use them — a few per message, not hundreds.
//! `accented_*` is the interesting one: it has no emoji at all, so every
//! codepoint is a miss, which is precisely the path with nothing to short-
//! circuit it.

use std::collections::HashMap;
use std::hint::black_box;

use client_core::events::CustomEmoji;
use criterion::{criterion_group, criterion_main, Criterion};
use ui::screens::timeline::{first_url_in, tokenize_line, unicode_emojis_in};

/// Plain ASCII, no colon, no link — the single most common message shape.
const ASCII_PLAIN: &str = "yeah that fix looks right to me, ship it when CI goes green";

/// Accented Latin, no emoji. Every non-ASCII char is an emoji-table miss.
const ACCENTED: &str = "Schöne Grüße aus München — das Wetter ist heute wirklich schön, \
                        wir sitzen draußen im Café und genießen die Sonne";

/// Cyrillic — same "non-ASCII but never an emoji" shape, different block.
const CYRILLIC: &str = "привет как дела у меня всё хорошо спасибо большое за помощь вчера";

/// A few emoji in an otherwise ordinary sentence — how emoji actually appear.
const EMOJI_SPRINKLED: &str = "nice work 🎉 that was a nasty one to track down 😅 merging now 🚀";

/// Emoji-only reaction-style message, incl. a ZWJ sequence and a keycap.
const EMOJI_ONLY: &str = "🎉🚀😅👨‍👩‍👧‍👦1️⃣🇩🇪";

/// A link mid-sentence.
const WITH_URL: &str = "see https://github.com/DDQW/thornychat/blob/master/README.md for the rationale";

/// Colons but no shortcode and no link — the case a naive ':' check trips on.
const TIMESTAMPY: &str = "standup moved to 12:30 tomorrow, and the deploy window is 18:00-19:45";

/// A real custom-emoji shortcode plus a near-miss that has to be rejected.
const SHORTCODES: &str = "that PR :thonk: took forever but :party_blob: we got there in the end";

fn shortcode_index() -> HashMap<String, CustomEmoji> {
    // Sized like a real pack set: a few packs' worth of shortcodes, so the
    // hash lookups hit a realistically-populated table.
    let mut index = HashMap::new();
    for i in 0..200 {
        index.insert(
            format!("pack_emoji_{i}"),
            CustomEmoji {
                shortcode: format!("pack_emoji_{i}"),
                mxc_url: format!("mxc://example.org/emoji{i}"),
                is_emoticon: true,
                is_sticker: false,
                width: Some(32),
                height: Some(32),
            },
        );
    }
    for code in ["thonk", "party_blob", "shrug", "blobcat"] {
        index.insert(
            code.to_string(),
            CustomEmoji {
                shortcode: code.to_string(),
                mxc_url: format!("mxc://example.org/{code}"),
                is_emoticon: true,
                is_sticker: false,
                width: Some(32),
                height: Some(32),
            },
        );
    }
    index
}

const CORPUS: &[(&str, &str)] = &[
    ("ascii_plain", ASCII_PLAIN),
    ("accented", ACCENTED),
    ("cyrillic", CYRILLIC),
    ("emoji_sprinkled", EMOJI_SPRINKLED),
    ("emoji_only", EMOJI_ONLY),
    ("with_url", WITH_URL),
    ("timestampy", TIMESTAMPY),
    ("shortcodes", SHORTCODES),
];

fn bench_unicode_emojis_in(c: &mut Criterion) {
    let mut group = c.benchmark_group("unicode_emojis_in");
    for (name, body) in CORPUS {
        group.bench_function(*name, |b| b.iter(|| unicode_emojis_in(black_box(body))));
    }
    group.finish();
}

fn bench_first_url_in(c: &mut Criterion) {
    let mut group = c.benchmark_group("first_url_in");
    for (name, body) in CORPUS {
        group.bench_function(*name, |b| b.iter(|| first_url_in(black_box(body))));
    }
    group.finish();
}

fn bench_tokenize_line(c: &mut Criterion) {
    let index = shortcode_index();
    let mut group = c.benchmark_group("tokenize_line");
    for (name, body) in CORPUS {
        group.bench_function(*name, |b| {
            b.iter(|| tokenize_line(black_box(body), black_box(&index)))
        });
    }
    group.finish();
}

/// A whole screen's worth of timeline in one shot — the number that actually
/// tracks a frame's text cost. Weighted toward the shapes that dominate real
/// scrollback rather than an even split across the corpus.
fn bench_screenful(c: &mut Criterion) {
    let index = shortcode_index();
    let screen: Vec<&str> = std::iter::repeat_n(
        [ASCII_PLAIN, ASCII_PLAIN, ACCENTED, ASCII_PLAIN, EMOJI_SPRINKLED, TIMESTAMPY, WITH_URL],
        6,
    )
    .flatten()
    .collect();

    c.bench_function("screenful_42_messages", |b| {
        b.iter(|| {
            for line in &screen {
                black_box(tokenize_line(black_box(line), black_box(&index)));
            }
        })
    });
}

criterion_group!(
    benches,
    bench_unicode_emojis_in,
    bench_first_url_in,
    bench_tokenize_line,
    bench_screenful
);
criterion_main!(benches);
