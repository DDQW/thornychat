//! The `tcmedia` URI scheme: how images, avatars, stickers, video and emoji
//! reach the page without their bytes ever crossing the IPC channel.
//!
//! The page's CSP allows images and media from this scheme only, so it is also
//! the single door through which anything remote is loaded:
//!
//! | Path | Served from |
//! |---|---|
//! | `/mxc/<server>/<media id>` | Matrix media, through the persistent disk cache (`client_core::media`) |
//! | `/twemoji/<codepoints>.svg` | Twemoji CDN, cached on disk |
//! | `/web?u=<https url>` | An allow-listed card image host (tweet / Steam art) |
//!
//! Either image route takes `upscale=<long edge>` for the lightbox: a
//! Lanczos3-upscaled copy, made and cached by `upscale.rs` (422 when there is
//! nothing to gain, 415 for animations and formats it can't decode — the page
//! then keeps the original).
//!
//! Matrix media is authenticated, so it can only be fetched by the client; the
//! page could never load an `mxc://` URL itself. Responses honour `Range`, which
//! the `<video>` element needs for seeking.

use std::borrow::Cow;
use std::io::SeekFrom;
use std::sync::Arc;

use tauri::http::{header, Request, Response, StatusCode};
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::bridge::Bridge;
use crate::embeds;

pub const SCHEME: &str = "tcmedia";

/// Adapter for `register_asynchronous_uri_scheme_protocol`.
pub fn handler<R: Runtime>(
    context: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let bridge = context.app_handle().state::<Arc<Bridge>>().inner().clone();
    tauri::async_runtime::spawn(async move {
        responder.respond(serve(&bridge, &request).await);
    });
}

#[derive(Debug, PartialEq, Eq)]
enum Route {
    Mxc { server: String, media_id: String },
    Twemoji(String),
    Web(String),
}

async fn serve(bridge: &Bridge, request: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let uri = request.uri();
    let Some(route) = parse_route(uri.path(), uri.query()) else {
        return status(StatusCode::NOT_FOUND, "unknown media route");
    };
    let range = request.headers().get(header::RANGE).and_then(|value| value.to_str().ok());
    let mime_hint = uri.query().and_then(|query| query_param(query, "mime"));
    let upscale = match uri.query().and_then(|query| query_param(query, "upscale")) {
        None => None,
        Some(edge) => match parse_upscale(&edge) {
            Some(edge) => Some(edge),
            None => return status(StatusCode::BAD_REQUEST, "bad upscale size"),
        },
    };

    match route {
        Route::Mxc { server, media_id } => {
            let Some(running) = bridge.running() else {
                return status(StatusCode::SERVICE_UNAVAILABLE, "not signed in");
            };
            let mxc = format!("mxc://{server}/{media_id}");
            if let Some(edge) = upscale {
                let load = || async {
                    let path = client_core::media::fetch_file(&running.client, &bridge.media_dir, &mxc).await?;
                    Ok(tokio::fs::read(path).await?)
                };
                return serve_upscaled(crate::upscale::upscaled(&bridge.upscale_dir, &mxc, edge, load).await).await;
            }
            match client_core::media::fetch_file(&running.client, &bridge.media_dir, &mxc).await {
                Ok(path) => serve_file(&path, range, mime_hint.as_deref()).await,
                Err(error) => {
                    tracing::debug!(%error, %mxc, "media fetch failed");
                    status(StatusCode::BAD_GATEWAY, "media unavailable")
                }
            }
        }
        Route::Twemoji(stem) => match embeds::twemoji::fetch(&bridge.emoji_dir, &stem).await {
            Ok(bytes) => respond_bytes(bytes, "image/svg+xml"),
            Err(error) => {
                tracing::debug!(%error, %stem, "twemoji fetch failed");
                status(StatusCode::NOT_FOUND, "no such emoji")
            }
        },
        Route::Web(url) if upscale.is_some() => {
            let load = || embeds::fetch_web_image(&bridge.emoji_dir, &url);
            let edge = upscale.unwrap_or_default();
            serve_upscaled(crate::upscale::upscaled(&bridge.upscale_dir, &url, edge, load).await).await
        }
        Route::Web(url) => match embeds::fetch_web_image(&bridge.emoji_dir, &url).await {
            Ok(bytes) => {
                let mime = sniff(&bytes, None);
                respond_bytes(bytes, &mime)
            }
            Err(error) => {
                tracing::debug!(%error, %url, "web image fetch failed");
                status(StatusCode::BAD_GATEWAY, "image unavailable")
            }
        },
    }
}

/// An `upscale=` long edge: a plain positive number no larger than the output
/// cap (the page asks for at most that; anything else is a malformed URL).
fn parse_upscale(value: &str) -> Option<u32> {
    let edge: u32 = value.parse().ok()?;
    (1..=crate::upscale::MAX_OUTPUT_EDGE).contains(&edge).then_some(edge)
}

async fn serve_upscaled(result: Result<std::path::PathBuf, crate::upscale::Refusal>) -> Response<Cow<'static, [u8]>> {
    use crate::upscale::Refusal;
    match result {
        Ok(path) => serve_file(&path, None, None).await,
        Err(Refusal::NotWorthIt) => status(StatusCode::UNPROCESSABLE_ENTITY, "nothing to gain from upscaling"),
        Err(Refusal::Animated | Refusal::Undecodable | Refusal::TooLarge) => status(StatusCode::UNSUPPORTED_MEDIA_TYPE, "not an image this can upscale"),
        Err(Refusal::Unavailable) => status(StatusCode::BAD_GATEWAY, "media unavailable"),
    }
}

fn parse_route(path: &str, query: Option<&str>) -> Option<Route> {
    let mut segments = path.trim_start_matches('/').split('/');
    match segments.next()? {
        "mxc" => {
            let server = segments.next()?;
            let media_id = segments.next()?;
            if segments.next().is_some() || !is_server_name(server) || !is_media_id(media_id) {
                return None;
            }
            Some(Route::Mxc { server: server.to_string(), media_id: media_id.to_string() })
        }
        "twemoji" => {
            let stem = segments.next()?.strip_suffix(".svg")?;
            if segments.next().is_some() || !embeds::twemoji::is_valid_codepoints(stem) {
                return None;
            }
            Some(Route::Twemoji(stem.to_string()))
        }
        "web" => {
            let url = query_param(query?, "u")?;
            embeds::is_allowed_web_image(&url).then_some(Route::Web(url))
        }
        _ => None,
    }
}

/// Matrix server names: a hostname or IP literal with an optional port.
/// Checked before the name is spliced into an `mxc://` URI.
fn is_server_name(name: &str) -> bool {
    // Starts like a hostname or an IPv6 literal, and has no empty labels —
    // which also rules out `.` and `..`.
    name.bytes().next().is_some_and(|first| first.is_ascii_alphanumeric() || first == b'[')
        && !name.contains("..")
        && name.len() <= 255
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']'))
}

fn is_media_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 255 && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

fn query_param(query: &str, key: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes()).find(|(name, _)| name == key).map(|(_, value)| value.into_owned())
}

/// Most bytes one open-ended range response (`bytes=N-`) carries. That is
/// what `<video>` asks for, at the start and after every seek; answered in
/// full, a 256 MiB clip meant a 256 MiB response body per request (measured),
/// copied again into the webview. A short 206 is ordinary HTTP — the player
/// asks for the next chunk when it gets there.
const MAX_OPEN_RANGE_BYTES: u64 = 4 * 1024 * 1024;

/// A byte range request resolved against a file length.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct ByteRange {
    start: u64,
    end: u64, // inclusive
}

/// What a `Range` header asks for, resolved against the file length.
#[derive(Debug, PartialEq, Eq)]
enum RangeRequest {
    /// No usable range: serve the whole file with 200.
    Full,
    Partial(ByteRange),
    /// A well-formed range that starts past the end of the file: 416.
    Unsatisfiable,
}

/// Resolves a single-range `Range: bytes=…` header. Multi-range requests,
/// other units and anything malformed are ignored (RFC 9110 lets a server do
/// exactly that), which browsers cope with by taking the whole file.
fn resolve_range(header: Option<&str>, len: u64) -> RangeRequest {
    let Some(spec) = header.and_then(|h| h.trim().strip_prefix("bytes=")) else {
        return RangeRequest::Full;
    };
    if spec.contains(',') {
        return RangeRequest::Full;
    }
    let Some((first, last)) = spec.split_once('-') else { return RangeRequest::Full };
    let (first, last) = (first.trim(), last.trim());

    let parsed = match (first.is_empty(), last.is_empty()) {
        // "-N": the last N bytes.
        (true, false) => last.parse::<u64>().ok().filter(|&n| n > 0).map(|n| (len.saturating_sub(n), len.saturating_sub(1))),
        // "N-": from N towards the end, a chunk at a time.
        (false, true) => first
            .parse::<u64>()
            .ok()
            .map(|start| (start, len.saturating_sub(1).min(start.saturating_add(MAX_OPEN_RANGE_BYTES - 1)))),
        (false, false) => match (first.parse::<u64>(), last.parse::<u64>()) {
            (Ok(start), Ok(end)) if start <= end => Some((start, end.min(len.saturating_sub(1)))),
            _ => None,
        },
        (true, true) => None,
    };
    match parsed {
        None => RangeRequest::Full,
        Some((start, _)) if len == 0 || start >= len => RangeRequest::Unsatisfiable,
        Some((start, end)) => RangeRequest::Partial(ByteRange { start, end }),
    }
}

async fn serve_file(path: &std::path::Path, range: Option<&str>, mime_hint: Option<&str>) -> Response<Cow<'static, [u8]>> {
    let Ok(mut file) = tokio::fs::File::open(path).await else {
        return status(StatusCode::NOT_FOUND, "media is not cached");
    };
    let Ok(len) = file.metadata().await.map(|meta| meta.len()) else {
        return status(StatusCode::INTERNAL_SERVER_ERROR, "could not stat media");
    };

    let mut head = vec![0u8; 4096.min(len as usize)];
    if file.read_exact(&mut head).await.is_err() {
        return status(StatusCode::INTERNAL_SERVER_ERROR, "could not read media");
    }
    let mime = sniff(&head, mime_hint);

    let wanted = match resolve_range(range, len) {
        RangeRequest::Full => None,
        RangeRequest::Partial(range) => Some(range),
        RangeRequest::Unsatisfiable => {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{len}"))
                .body(Cow::Borrowed(&[][..]))
                .expect("static response parts are valid");
        }
    };

    let (start, end) = wanted.map_or((0, len.saturating_sub(1)), |r| (r.start, r.end));
    let count = if len == 0 { 0 } else { end - start + 1 };
    let mut body = vec![0u8; count as usize];
    if count > 0
        && (file.seek(SeekFrom::Start(start)).await.is_err() || file.read_exact(&mut body).await.is_err())
    {
        return status(StatusCode::INTERNAL_SERVER_ERROR, "could not read media");
    }

    let mut response = base_response(&mime)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, count);
    if wanted.is_some() {
        response = response
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"));
    }
    response.body(Cow::Owned(body)).expect("static response parts are valid")
}

fn respond_bytes(bytes: Vec<u8>, mime: &str) -> Response<Cow<'static, [u8]>> {
    base_response(mime)
        .header(header::CONTENT_LENGTH, bytes.len())
        .body(Cow::Owned(bytes))
        .expect("static response parts are valid")
}

/// Headers every successful response carries. The CSP sandbox and `nosniff`
/// are belt-and-braces: the page only ever embeds these URLs in `<img>`,
/// `<video>` and `<audio>`, but the bytes come from other people, so a stray
/// navigation to one must not be able to run anything.
fn base_response(mime: &str) -> tauri::http::response::Builder {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header("X-Content-Type-Options", "nosniff")
        .header("Content-Security-Policy", "default-src 'none'; sandbox")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        // An mxc URI / codepoint sequence names immutable content.
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
}

fn status(code: StatusCode, message: &'static str) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(code)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header("X-Content-Type-Options", "nosniff")
        .body(Cow::Borrowed(message.as_bytes()))
        .expect("static response parts are valid")
}

/// The content type to serve: sniffed from the bytes, falling back to the
/// sender-declared type (`?mime=`) the page passes along for video. Only
/// image, video and audio types ever come out — everything else is opaque.
fn sniff(head: &[u8], hint: Option<&str>) -> String {
    let sniffed = infer::get(head).map(|kind| kind.mime_type().to_string()).or_else(|| {
        let text = String::from_utf8_lossy(&head[..head.len().min(512)]);
        let trimmed = text.trim_start();
        (trimmed.starts_with("<svg") || (trimmed.starts_with("<?xml") && trimmed.contains("<svg")))
            .then(|| "image/svg+xml".to_string())
    });
    let hinted = hint.filter(|hint| is_safe_media_type(hint)).map(str::to_string);
    let mime = sniffed.filter(|mime| is_safe_media_type(mime)).or(hinted);
    mime.unwrap_or_else(|| "application/octet-stream".to_string())
}

fn is_safe_media_type(mime: &str) -> bool {
    let Some((kind, subtype)) = mime.split_once('/') else { return false };
    matches!(kind, "image" | "video" | "audio")
        && !subtype.is_empty()
        && subtype.len() <= 64
        && subtype.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_are_parsed_and_validated() {
        assert_eq!(
            parse_route("/mxc/matrix.org/TQtMBGzGjgtdYfjYIMtXSLjZ", None),
            Some(Route::Mxc { server: "matrix.org".into(), media_id: "TQtMBGzGjgtdYfjYIMtXSLjZ".into() })
        );
        assert_eq!(parse_route("/twemoji/1f600.svg", None), Some(Route::Twemoji("1f600".into())));
        assert_eq!(
            parse_route("/web", Some("u=https%3A%2F%2Fpbs.twimg.com%2Fa.jpg")),
            Some(Route::Web("https://pbs.twimg.com/a.jpg".into()))
        );
    }

    #[test]
    fn upscale_sizes_are_bounded() {
        assert_eq!(parse_upscale("2048"), Some(2048));
        assert_eq!(parse_upscale("4096"), Some(4096));
        for bad in ["0", "4097", "-1", "1e3", "", "99999999999", "2048px"] {
            assert_eq!(parse_upscale(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn hostile_routes_are_refused() {
        for (path, query) in [
            ("/mxc/matrix.org", None),
            ("/mxc/matrix.org/a/b", None),
            ("/mxc/ma trix.org/abc", None),
            ("/mxc/matrix.org/a.b", None),
            ("/mxc/matrix.org/..", None),
            ("/mxc/../abc", None),
            ("/twemoji/../../x.svg", None),
            ("/twemoji/1f600", None),
            ("/web", None),
            ("/web", Some("u=https%3A%2F%2Fevil.example%2Fa.jpg")),
            ("/web", Some("u=file%3A%2F%2F%2FC%3A%2Fsecret")),
            ("/other", None),
            ("/", None),
        ] {
            assert_eq!(parse_route(path, query), None, "{path} {query:?}");
        }
    }

    fn partial(start: u64, end: u64) -> RangeRequest {
        RangeRequest::Partial(ByteRange { start, end })
    }

    #[test]
    fn ranges_resolve_against_the_file_length() {
        assert_eq!(resolve_range(Some("bytes=0-99"), 1000), partial(0, 99));
        assert_eq!(resolve_range(Some("bytes=900-"), 1000), partial(900, 999));
        assert_eq!(resolve_range(Some("bytes=-100"), 1000), partial(900, 999));
        assert_eq!(resolve_range(Some("bytes=990-5000"), 1000), partial(990, 999));
        assert_eq!(resolve_range(Some("bytes=0-0"), 1000), partial(0, 0));
        assert_eq!(resolve_range(Some("bytes=-5000"), 1000), partial(0, 999));
    }

    #[test]
    fn open_ended_ranges_are_served_a_chunk_at_a_time() {
        let len = 256 * 1024 * 1024;
        assert_eq!(resolve_range(Some("bytes=0-"), len), partial(0, MAX_OPEN_RANGE_BYTES - 1));
        let middle = 100 * 1024 * 1024;
        assert_eq!(resolve_range(Some("bytes=104857600-"), len), partial(middle, middle + MAX_OPEN_RANGE_BYTES - 1));
        // Near the end, the chunk stops at the last byte.
        assert_eq!(resolve_range(Some(&format!("bytes={}-", len - 10)), len), partial(len - 10, len - 1));
        // An explicit range is honoured as asked.
        assert_eq!(resolve_range(Some("bytes=0-10485759"), len), partial(0, 10 * 1024 * 1024 - 1));
    }

    #[tokio::test]
    async fn a_chunked_response_says_where_it_ends() {
        let path = std::env::temp_dir().join(format!("thornychat-range-test-{}", std::process::id()));
        std::fs::write(&path, vec![1u8; (MAX_OPEN_RANGE_BYTES + 1000) as usize]).unwrap();
        let response = serve_file(&path, Some("bytes=0-"), Some("video/mp4")).await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.body().len() as u64, MAX_OPEN_RANGE_BYTES);
        let expected = format!("bytes 0-{}/{}", MAX_OPEN_RANGE_BYTES - 1, MAX_OPEN_RANGE_BYTES + 1000);
        assert_eq!(response.headers().get(header::CONTENT_RANGE).unwrap(), expected.as_str());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_range_past_the_end_is_unsatisfiable() {
        assert_eq!(resolve_range(Some("bytes=1000-"), 1000), RangeRequest::Unsatisfiable);
        assert_eq!(resolve_range(Some("bytes=2000-3000"), 1000), RangeRequest::Unsatisfiable);
        assert_eq!(resolve_range(Some("bytes=0-"), 0), RangeRequest::Unsatisfiable);
    }

    #[test]
    fn unsupported_or_malformed_ranges_serve_the_whole_file() {
        for header in [None, Some("bytes=50-10"), Some("bytes=0-1,5-6"), Some("bytes=-0"), Some("items=0-1"), Some("bytes=abc-"), Some("bytes=-"), Some("garbage")] {
            assert_eq!(resolve_range(header, 1000), RangeRequest::Full, "{header:?}");
        }
    }

    #[test]
    fn content_types_come_from_the_bytes_first() {
        let png = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];
        assert_eq!(sniff(&png, None), "image/png");
        // The sender's claim doesn't override what the bytes say.
        assert_eq!(sniff(&png, Some("video/mp4")), "image/png");
        assert_eq!(sniff(b"<svg xmlns='http://www.w3.org/2000/svg'/>", None), "image/svg+xml");
    }

    #[test]
    fn unknown_bytes_use_a_safe_hint_or_stay_opaque() {
        let junk = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        assert_eq!(sniff(&junk, Some("video/mp4")), "video/mp4");
        assert_eq!(sniff(&junk, Some("text/html")), "application/octet-stream");
        assert_eq!(sniff(&junk, Some("video/mp4; x=y")), "application/octet-stream");
        assert_eq!(sniff(&junk, None), "application/octet-stream");
    }

    /// What one `<video>` request costs: `cargo test -p thornychat --release open_ended_range_cost -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn open_ended_range_cost() {
        let path = std::env::temp_dir().join(format!("thornychat-range-bench-{}", std::process::id()));
        std::fs::write(&path, vec![7u8; 256 * 1024 * 1024]).unwrap();
        let started = std::time::Instant::now();
        let response = serve_file(&path, Some("bytes=0-"), Some("video/mp4")).await;
        println!(
            "bytes=0- on a 256 MiB file: {:?}, body {} MiB, Content-Range {:?}",
            started.elapsed(),
            response.body().len() / (1024 * 1024),
            response.headers().get(header::CONTENT_RANGE)
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn html_is_never_served_as_html() {
        assert_eq!(sniff(b"<!DOCTYPE html><html><script>alert(1)</script>", None), "application/octet-stream");
    }
}
