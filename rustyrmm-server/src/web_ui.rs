//! The web UI (`rustyrmm-web/dist`), embedded in the binary and served from the Operator port
//! behind the gRPC routes.

use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

#[derive(rust_embed::RustEmbed)]
#[folder = "../rustyrmm-web/dist"]
// Debug builds compile without the UI and serve a placeholder; build.rs makes release builds
// fail instead, so a shipped Server always has it.
#[allow_missing = true]
struct Assets;

/// Same-origin only: the page talks to this Server and nothing else, and may not be framed.
const CSP: &str = "default-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

const PLACEHOLDER: &str = "<!doctype html><title>RustyRMM</title><p>The web UI is not built. \
    Run <code>npm ci &amp;&amp; npm run build</code> in <code>rustyrmm-web/</code> and rebuild the Server.</p>";

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    // Unknown paths without a file extension are app routes: serve the app.
    let file = Assets::get(path).map(|f| (path, f)).or_else(|| {
        (!path.contains('.'))
            .then(|| Assets::get("index.html").map(|f| ("index.html", f)))
            .flatten()
    });
    let mut response = match file {
        Some((name, file)) => {
            let mime = file.metadata.mimetype().to_owned();
            // Vite puts content hashes in asset names, so those never change; index.html may.
            let cache = if name.starts_with("assets/") {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            (
                [
                    (header::CONTENT_TYPE, mime),
                    (header::CACHE_CONTROL, cache.to_owned()),
                ],
                file.data,
            )
                .into_response()
        }
        None if Assets::get("index.html").is_none() => (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            PLACEHOLDER,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    };
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}
