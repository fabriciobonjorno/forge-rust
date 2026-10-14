//! Rendering Askama views into HTTP responses.

use askama::Template;
use forge_http::{HeaderValue, Response, StatusCode, header};
use tracing::error;

/// The media type of every HTML response.
pub(crate) const HTML_CONTENT_TYPE: &str = "text/html; charset=utf-8";

/// Renders `view` as a `200 OK` HTML response.
///
/// Templates with an `.html` extension are HTML-escaped by Askama by default:
/// every `{{ value }}` is escaped, and raw output requires an explicit
/// `|safe`. A render failure answers `500` with an empty body and logs the
/// error; template error text never reaches the client.
///
/// ```
/// use askama::Template;
///
/// #[derive(Template)]
/// #[template(source = "<h1>Hello, {{ name }}</h1>", ext = "html")]
/// struct Hello<'a> {
///     name: &'a str,
/// }
///
/// let response = forge_views::html(&Hello { name: "<script>" });
/// assert_eq!(response.status(), forge_http::StatusCode::OK);
/// assert_eq!(response.body().as_ref(), b"<h1>Hello, &#60;script&#62;</h1>");
/// ```
#[must_use]
pub fn html(view: &impl Template) -> Response {
    html_with_status(StatusCode::OK, view)
}

/// Renders `view` as an HTML response with `status`, for example `422
/// Unprocessable Entity` when a form is re-rendered with validation errors.
/// Failure handling is the same as [`html`]: `500`, empty body, error logged.
#[must_use]
pub fn html_with_status(status: StatusCode, view: &impl Template) -> Response {
    match view.render() {
        Ok(body) => {
            let mut response = Response::new(status, body);
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static(HTML_CONTENT_TYPE),
            );
            response
        }
        Err(failure) => {
            error!(error = %failure, "view rendering failed");
            Response::empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fmt;

    use super::*;
    use crate::test_support::{header as header_of, text};

    #[derive(Template)]
    #[template(source = "<p>{{ name }}</p>", ext = "html")]
    struct Greeting<'a> {
        name: &'a str,
    }

    #[derive(Template)]
    #[template(source = "<p>{{ name|safe }}</p>", ext = "html")]
    struct RawGreeting<'a> {
        name: &'a str,
    }

    struct Failing;

    impl fmt::Display for Failing {
        fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
            Err(fmt::Error)
        }
    }

    #[derive(Template)]
    #[template(source = "<p>internal-template-detail {{ value }}</p>", ext = "html")]
    struct Broken {
        value: Failing,
    }

    #[test]
    fn renders_ok_html_with_the_html_content_type() {
        let response = html(&Greeting { name: "Ada" });

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(text(&response), "<p>Ada</p>");
        assert_eq!(
            header_of(&response, "content-type"),
            Some("text/html; charset=utf-8")
        );
    }

    #[test]
    fn html_with_status_keeps_the_requested_status() {
        let response = html_with_status(StatusCode::UNPROCESSABLE_ENTITY, &Greeting { name: "x" });

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            header_of(&response, "content-type"),
            Some("text/html; charset=utf-8")
        );
    }

    #[test]
    fn fields_are_html_escaped_by_default() {
        let response = html(&Greeting {
            name: "<script>alert('x')</script>&\"",
        });

        let body = text(&response);
        assert!(!body.contains("<script>"), "{body}");
        assert!(body.contains("&#60;script&#62;"), "{body}");
        assert!(body.contains("&#38;"), "{body}");
        assert!(body.contains("&#34;"), "{body}");
    }

    #[test]
    fn raw_output_requires_an_explicit_safe_filter() {
        let response = html(&RawGreeting { name: "<b>ok</b>" });

        assert_eq!(text(&response), "<p><b>ok</b></p>");
    }

    #[test]
    fn render_failures_answer_a_generic_empty_500() {
        let response = html(&Broken { value: Failing });

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(response.body().is_empty());
        assert_eq!(header_of(&response, "content-type"), None);
    }

    #[test]
    fn render_failures_ignore_the_requested_status() {
        let response = html_with_status(StatusCode::CREATED, &Broken { value: Failing });

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(response.body().is_empty());
    }
}
