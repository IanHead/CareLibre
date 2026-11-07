//! Double-submit CSRF protection for admin forms.
//!
//! The token lives in an HttpOnly cookie and is echoed in a hidden form field.
//! Server-rendered forms mean no JavaScript is needed to read it.

use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};

pub const COOKIE_NAME: &str = "carelibre_csrf";

/// Return an existing token or issue a new one. The returned jar must be sent
/// with the response.
pub fn ensure(jar: CookieJar, secure: bool) -> (CookieJar, String) {
    if let Some(existing) = jar.get(COOKIE_NAME) {
        let token = existing.value().to_owned();
        return (jar, token);
    }

    let token = crate::auth::refresh::generate().0;
    let cookie = Cookie::build((COOKIE_NAME, token.clone()))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .secure(secure)
        .build();

    (jar.add(cookie), token)
}

#[must_use]
pub fn verify(jar: &CookieJar, submitted: &str) -> bool {
    jar.get(COOKIE_NAME)
        .map(|cookie| !submitted.is_empty() && cookie.value() == submitted)
        .unwrap_or(false)
}
