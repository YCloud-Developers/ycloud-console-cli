use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};

const TEMPLATE: &str = include_str!("oauth_callback/index.html");
const STYLE: &str = include_str!("oauth_callback/style.css");
const SCRIPT: &str = include_str!("oauth_callback/page.js");
const LOGO: &[u8] = include_bytes!("oauth_callback/logo.png");

pub(crate) enum PageState {
    Received,
    Denied,
    Rejected,
}

// Only a fixed state reaches the template: callback query parameters and credentials
// must never be reflected into this local browser response.
pub(crate) fn response(state: PageState) -> String {
    let (status, name, title, description) = match state {
        PageState::Received => (
            "200 OK", "received", "Browser authorization complete",
            "Return to your terminal to confirm login.",
        ),
        PageState::Denied => (
            "400 Bad Request", "denied", "Authorization cancelled",
            "You cancelled this authorization. YCloud CLI will not gain access through this request.",
        ),
        PageState::Rejected => (
            "400 Bad Request", "rejected", "Unable to verify this callback",
            "This request could not be verified. Check your terminal or start login again.",
        ),
    };
    let body = TEMPLATE
        .replace("__YC_STATE__", name)
        .replace("__YC_SYMBOL__", if name == "received" { "✓" } else { "!" })
        .replace("__YC_TITLE__", title)
        .replace("__YC_DESCRIPTION__", description)
        .replace("__YC_LOGO__", &STANDARD.encode(LOGO))
        .replace("__YC_STYLE__", STYLE)
        .replace("__YC_SCRIPT__", SCRIPT);
    let style_hash = STANDARD.encode(Sha256::digest(STYLE.as_bytes()));
    let script_hash = STANDARD.encode(Sha256::digest(SCRIPT.as_bytes()));
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'none'; img-src data:; style-src 'sha256-{style_hash}'; script-src 'sha256-{script_hash}'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
