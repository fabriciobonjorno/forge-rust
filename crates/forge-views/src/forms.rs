//! Form helpers: CSRF markup and HTTP method tunnelling.

use forge_auth::CsrfToken;
use forge_http::Method;

/// Header that carries the CSRF token on `fetch`/Turbo requests. This is the
/// header the Phase 3 enforcement in generated applications reads
/// (`x-csrf-token`).
pub const CSRF_HEADER_NAME: &str = "x-csrf-token";

/// `name` of the `<meta>` element that exposes the token to Turbo and
/// `fetch` (ADR 0022).
pub const CSRF_META_NAME: &str = "csrf-token";

/// `name` of the hidden form field that carries the token in HTML form
/// submissions. The server-side CSRF check for form posts must read this
/// field with the same rules as [`CSRF_HEADER_NAME`].
pub const CSRF_FIELD_NAME: &str = "csrf_token";

/// `name` of the form field that tunnels `PATCH`, `PUT` and `DELETE`
/// through `POST`.
pub const METHOD_FIELD_NAME: &str = "_method";

/// `<meta name="csrf-token" content="...">` for the layout `<head>`, so
/// Turbo and scripts can send the token in [`CSRF_HEADER_NAME`].
///
/// The result is markup: insert it with `|safe` in a template. The value is
/// HTML-escaped here even though a [`CsrfToken`] is hex by construction.
#[must_use]
pub fn csrf_meta(token: &CsrfToken) -> String {
    meta_markup(token.expose())
}

/// `<input type="hidden" name="csrf_token" value="...">` for every non-GET
/// form. Insert it with `|safe` in a template.
#[must_use]
pub fn csrf_field(token: &CsrfToken) -> String {
    field_markup(token.expose())
}

fn meta_markup(value: &str) -> String {
    format!(
        "<meta name=\"{CSRF_META_NAME}\" content=\"{}\">",
        escape_attribute(value)
    )
}

fn field_markup(value: &str) -> String {
    format!(
        "<input type=\"hidden\" name=\"{CSRF_FIELD_NAME}\" value=\"{}\">",
        escape_attribute(value)
    )
}

/// Escapes text for a double- or single-quoted HTML attribute (and for text
/// content).
fn escape_attribute(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Proof, at the call site, that the request's CSRF token has already been
/// validated.
///
/// Method tunnelling turns a `POST` into a state-changing `PATCH`, `PUT` or
/// `DELETE`; it must never be honored for a request whose token has not been
/// checked, or a cross-site form could trigger deletions. This witness is
/// required by [`method_override`] so that contract is visible in the code
/// that calls it. Construct it only in the code path that follows a
/// successful check.
#[derive(Clone, Copy, Debug)]
pub struct CsrfVerified(());

impl CsrfVerified {
    /// Records that CSRF validation succeeded for the current request.
    ///
    /// Call this only after the CSRF token (hidden field or header) has been
    /// verified against the session.
    #[must_use]
    pub fn after_check() -> Self {
        Self(())
    }
}

/// The method a form submission should be routed as.
///
/// Browsers submit HTML forms as `GET` or `POST` only. A form can carry the
/// real method in a [`METHOD_FIELD_NAME`] (`_method`) field; this function
/// applies it under strict rules:
///
/// - only when the request method is `POST` (a `GET` with `?_method=DELETE`
///   is never honored);
/// - only the values `PATCH`, `PUT` and `DELETE`, compared ASCII
///   case-insensitively (`POST`, `GET`, `TRACE`, unknown or empty values leave
///   the request as `POST`);
/// - only after CSRF validation, which the [`CsrfVerified`] witness makes
///   explicit.
///
/// `field` is the decoded value of the `_method` form field, if present.
///
/// ```
/// use forge_http::Method;
/// use forge_views::{CsrfVerified, method_override};
///
/// let verified = CsrfVerified::after_check();
/// assert_eq!(method_override(verified, &Method::POST, Some("delete")), Method::DELETE);
/// assert_eq!(method_override(verified, &Method::GET, Some("DELETE")), Method::GET);
/// ```
#[must_use]
pub fn method_override(_verified: CsrfVerified, method: &Method, field: Option<&str>) -> Method {
    if *method != Method::POST {
        return method.clone();
    }
    match field {
        Some(value) if value.eq_ignore_ascii_case("PATCH") => Method::PATCH,
        Some(value) if value.eq_ignore_ascii_case("PUT") => Method::PUT,
        Some(value) if value.eq_ignore_ascii_case("DELETE") => Method::DELETE,
        _ => Method::POST,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn token() -> CsrfToken {
        CsrfToken::new(TOKEN).expect("valid token")
    }

    #[test]
    fn meta_and_field_render_the_token() {
        assert_eq!(
            csrf_meta(&token()),
            format!("<meta name=\"csrf-token\" content=\"{TOKEN}\">")
        );
        assert_eq!(
            csrf_field(&token()),
            format!("<input type=\"hidden\" name=\"csrf_token\" value=\"{TOKEN}\">")
        );
    }

    #[test]
    fn markup_escapes_attribute_breaking_values() {
        let hostile = "\"><script>alert('x')&</script>";

        let meta = meta_markup(hostile);
        let field = field_markup(hostile);

        for markup in [&meta, &field] {
            assert!(!markup.contains("<script>"), "{markup}");
            assert!(!markup.contains("\"><"), "{markup}");
            assert!(
                markup.contains("&quot;&gt;&lt;script&gt;alert(&#39;x&#39;)&amp;&lt;/script&gt;"),
                "{markup}"
            );
        }
        assert_eq!(meta.matches('"').count(), 4);
        assert_eq!(field.matches('"').count(), 6);
    }

    #[test]
    fn header_name_matches_the_generated_application_check() {
        assert_eq!(CSRF_HEADER_NAME, "x-csrf-token");
        assert_eq!(CSRF_META_NAME, "csrf-token");
    }

    fn resolve(method: Method, field: Option<&str>) -> Method {
        method_override(CsrfVerified::after_check(), &method, field)
    }

    #[test]
    fn post_with_patch_put_delete_is_honored() {
        assert_eq!(resolve(Method::POST, Some("PATCH")), Method::PATCH);
        assert_eq!(resolve(Method::POST, Some("PUT")), Method::PUT);
        assert_eq!(resolve(Method::POST, Some("DELETE")), Method::DELETE);
        assert_eq!(resolve(Method::POST, Some("delete")), Method::DELETE);
        assert_eq!(resolve(Method::POST, Some("Put")), Method::PUT);
    }

    #[test]
    fn non_post_requests_ignore_the_field() {
        for method in [
            Method::GET,
            Method::HEAD,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ] {
            assert_eq!(resolve(method.clone(), Some("DELETE")), method);
        }
    }

    #[test]
    fn other_values_leave_post_as_post() {
        for value in [
            Some("GET"),
            Some("POST"),
            Some("TRACE"),
            Some("CONNECT"),
            Some("HEAD"),
            Some("OPTIONS"),
            Some("DELETE "),
            Some(" DELETE"),
            Some("DELETE\n"),
            Some("DELET"),
            Some("custom"),
            Some(""),
            None,
        ] {
            assert_eq!(resolve(Method::POST, value), Method::POST, "{value:?}");
        }
    }
}
