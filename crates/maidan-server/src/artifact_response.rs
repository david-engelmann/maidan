//! How artifact bytes go out over HTTP, and what an upload may call them.
//!
//! The bytes are whatever a member uploaded, so the response is built as if
//! they were hostile: the content type is the one stored at upload, never
//! sniffed; nothing in them may run; and only raster images render in place.

use axum::{
    http::{header, HeaderMap, HeaderName, HeaderValue},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use maidan_types::Artifact;

use crate::error::ApiError;

/// Types a browser may render in place. SVG is not one: it is a document that
/// can carry script, and the `/ui` shows images through `blob:` URLs, which
/// keep the page's origin and drop this response's CSP. A PNG opened that way
/// is still only pixels; an SVG would run in the console's origin.
pub const INLINE_IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Opened directly, the bytes are a sandboxed document that can load nothing
/// but itself: no script, no requests, no forms.
pub const ARTIFACT_CSP: &str =
    "default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'; sandbox";

const FALLBACK_TYPE: &str = "application/octet-stream";

/// Longest filename an upload may give, in characters.
pub const MAX_FILENAME_CHARS: usize = 255;

/// The type to serve: the stored type's `type/subtype`, lowercased, or
/// `application/octet-stream` when none was stored or it is not a media type.
/// Parameters are dropped; nothing served here needs a charset to be safe.
pub fn served_type(stored: Option<&str>) -> String {
    let Some(stored) = stored else {
        return FALLBACK_TYPE.into();
    };
    let essence = stored
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let valid = essence.split_once('/').is_some_and(|(kind, sub)| {
        !kind.is_empty() && !sub.is_empty() && kind.bytes().chain(sub.bytes()).all(is_tchar)
    });
    if valid {
        essence
    } else {
        FALLBACK_TYPE.into()
    }
}

/// RFC 9110 `tchar`: what a media type's type and subtype are made of.
fn is_tchar(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

pub fn renders_inline(served: &str) -> bool {
    INLINE_IMAGE_TYPES.contains(&served)
}

/// `inline` or `attachment`, with the name when there is one: an ASCII
/// `filename` with anything unusual replaced, and the exact name
/// percent-encoded in `filename*` (RFC 6266), so no quote, semicolon or
/// newline in a name can reach the header's syntax.
pub fn content_disposition(inline: bool, filename: Option<&str>) -> HeaderValue {
    let disposition = if inline { "inline" } else { "attachment" };
    let Some(name) = filename else {
        return HeaderValue::from_static(disposition);
    };
    let ascii: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || " .-_()".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    HeaderValue::from_str(&format!(
        "{disposition}; filename=\"{ascii}\"; filename*=UTF-8''{encoded}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}

/// The bytes of `artifact`, described by the metadata the caller's workspace
/// stored for them.
pub fn artifact_response(artifact: &Artifact, bytes: Bytes) -> Response {
    let served = served_type(artifact.mime_type.as_deref());
    let inline = renders_inline(&served);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&served).unwrap_or(HeaderValue::from_static(FALLBACK_TYPE)),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(ARTIFACT_CSP),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        content_disposition(inline, artifact.filename.as_deref()),
    );
    // The headers are one workspace's view of shared bytes: no cache may hand
    // them to another requester.
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("private"));
    if let Ok(kind) = artifact.kind.as_str().parse() {
        headers.insert(HeaderName::from_static("x-artifact-kind"), kind);
    }
    (headers, bytes).into_response()
}

/// Bidirectional controls reorder how a name displays: `photo\u{202E}gpj.exe`
/// reads as `photoexe.jpg`.
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// The display name to store for an upload. A path keeps only its last
/// segment: a filename names, it never locates. A name that is empty, `.` or
/// `..` once reduced is no name. Control and bidirectional-control characters
/// are refused rather than stripped, so what is stored is what was sent.
pub fn upload_filename(raw: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.chars().any(|c| c.is_control() || is_bidi_control(c)) {
        return Err(ApiError::BadRequest(
            "filename contains a control or bidirectional-control character".into(),
        ));
    }
    let name = raw.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    if name.is_empty() || name == "." || name == ".." {
        return Ok(None);
    }
    if name.chars().count() > MAX_FILENAME_CHARS {
        return Err(ApiError::BadRequest(format!(
            "filename is longer than {MAX_FILENAME_CHARS} characters"
        )));
    }
    Ok(Some(name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn served_type_is_the_stored_essence_or_octet_stream() {
        assert_eq!(served_type(Some("Image/PNG; foo=bar")), "image/png");
        assert_eq!(served_type(None), FALLBACK_TYPE);
        assert_eq!(served_type(Some("not a type")), FALLBACK_TYPE);
        assert_eq!(served_type(Some("text/html\r\nX: y")), FALLBACK_TYPE);
        assert_eq!(served_type(Some("/png")), FALLBACK_TYPE);
    }

    #[test]
    fn only_raster_images_render_inline() {
        for inline in INLINE_IMAGE_TYPES {
            assert!(renders_inline(inline));
        }
        for other in [
            "image/svg+xml",
            "text/html",
            "application/pdf",
            FALLBACK_TYPE,
        ] {
            assert!(!renders_inline(other), "{other}");
        }
    }

    #[test]
    fn a_hostile_filename_cannot_reach_the_header_syntax() {
        let value = content_disposition(false, Some("a\";b=c; x.png"));
        let value = value.to_str().unwrap();
        assert_eq!(
            value,
            "attachment; filename=\"a__b_c_ x.png\"; filename*=UTF-8''a%22%3Bb%3Dc%3B%20x.png"
        );
        let value = content_disposition(true, Some("café.png"));
        assert_eq!(
            value.to_str().unwrap(),
            "inline; filename=\"caf_.png\"; filename*=UTF-8''caf%C3%A9.png"
        );
        assert_eq!(content_disposition(true, None), "inline");
    }

    #[test]
    fn an_upload_filename_keeps_its_last_segment_and_refuses_controls() {
        assert_eq!(
            upload_filename(Some("../../etc/evil.png".into())).unwrap(),
            Some("evil.png".into())
        );
        assert_eq!(
            upload_filename(Some("C:\\Users\\me\\shot.png".into())).unwrap(),
            Some("shot.png".into())
        );
        assert_eq!(upload_filename(Some("dir/..".into())).unwrap(), None);
        assert_eq!(upload_filename(Some("  ".into())).unwrap(), None);
        assert!(upload_filename(Some("a\nb.png".into())).is_err());
        assert!(upload_filename(Some("photo\u{202E}gpj.exe".into())).is_err());
        assert!(upload_filename(Some("x".repeat(MAX_FILENAME_CHARS + 1))).is_err());
        assert!(upload_filename(Some("é".repeat(MAX_FILENAME_CHARS))).is_ok());
    }
}
