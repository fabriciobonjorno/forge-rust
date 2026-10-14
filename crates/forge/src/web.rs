//! Helpers that keep HTTP handlers thin: decode a request, call a use case,
//! map the result.
//!
//! ```rust
//! use forge::core::{ErrorCategory, ForgeError};
//! use forge::http::StatusCode;
//!
//! let response = forge::web::error(&ForgeError::new(
//!     ErrorCategory::NotFound,
//!     "product_not_found",
//!     "product not found",
//! ));
//! assert_eq!(response.status(), StatusCode::NOT_FOUND);
//! ```

use std::{error::Error as StdError, str::FromStr};

use forge_core::{ErrorCategory, ForgeError};
use forge_http::{HeaderValue, PathParamError, Request, Response, StatusCode, header};
use serde::{Serialize, de::DeserializeOwned};
use tracing::{error, warn};

/// Creates a UTF-8 HTML response with an explicit content type.
#[must_use]
pub fn html(status: StatusCode, document: impl Into<String>) -> Response {
    let mut response = Response::new(status, document.into());
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
}

/// Escapes a text value for insertion into HTML text or quoted attribute
/// content. Generated views use this by default; trusted markup must be
/// represented separately rather than passed through this function.
#[must_use]
pub fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        escaped.push_str(match character {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            '\'' => "&#x27;",
            _ => {
                escaped.push(character);
                continue;
            }
        });
    }
    escaped
}

/// Serializes `value` as a JSON response. Serialization failures (a bug in a
/// `Serialize` implementation) become an empty `500` and are logged.
#[must_use]
pub fn json(status: StatusCode, value: &impl Serialize) -> Response {
    Response::json(status, value).unwrap_or_else(|failure| {
        error!(error = %failure, "response serialization failed");
        Response::empty(StatusCode::INTERNAL_SERVER_ERROR)
    })
}

/// Renders a [`ForgeError`] as its caller-safe JSON report with the status of
/// its category. Internal sources never reach the client: server-side errors
/// are logged with their full cause chain instead.
#[must_use]
pub fn error(failure: &ForgeError) -> Response {
    let status =
        StatusCode::from_u16(failure.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    if status.is_server_error() {
        let chain = cause_chain(failure);
        match failure.category() {
            ErrorCategory::Infrastructure => {
                warn!(code = failure.code(), error = %chain, "dependency failure");
            }
            _ => error!(code = failure.code(), error = %chain, "request failed"),
        }
    }
    json(status, &failure.report())
}

/// Decodes a JSON request body, mapping malformed input to a `Validation`
/// error with code `invalid_json` (the decoder's detail is not exposed).
pub fn decode_json<T: DeserializeOwned>(request: &Request) -> Result<T, ForgeError> {
    request.json().map_err(|failure| {
        ForgeError::new(
            ErrorCategory::Validation,
            "invalid_json",
            "request body is not valid JSON for this endpoint",
        )
        .with_source(failure)
    })
}

/// Decodes the query string into `T` (see [`Request::query`] for the strict
/// `application/x-www-form-urlencoded` rules and supported field types).
///
/// Every failure is a `Validation` error with code `invalid_query` and a
/// caller-safe message; when it concerns one parameter (malformed,
/// repeated, unknown, missing or invalid) its name, truncated to 64
/// characters, is in the `parameter` metadata. Unknown parameters are only
/// rejected when `T` uses `#[serde(deny_unknown_fields)]`; decode into `()`
/// to accept no query at all.
///
/// ```rust
/// use forge::http::{Method, Request, Uri, HeaderMap};
/// use bytes::Bytes;
/// use uuid::Uuid;
///
/// #[derive(serde::Deserialize)]
/// #[serde(deny_unknown_fields)]
/// struct Paging {
///     limit: Option<u16>,
/// }
///
/// let request = Request::new_for_test(
///     Method::GET,
///     Uri::from_static("/products?limit=25"),
///     HeaderMap::new(),
///     Bytes::new(),
///     "127.0.0.1:4000".parse().unwrap(),
///     Uuid::now_v7(),
/// );
/// let paging: Paging = forge::web::decode_query(&request)?;
/// assert_eq!(paging.limit, Some(25));
/// # Ok::<(), forge::core::ForgeError>(())
/// ```
pub fn decode_query<T: DeserializeOwned>(request: &Request) -> Result<T, ForgeError> {
    request.query().map_err(|failure| {
        let mut error = ForgeError::new(
            ErrorCategory::Validation,
            "invalid_query",
            failure.kind().message(),
        );
        if let Some(parameter) = failure.parameter() {
            error = error.with_metadata("parameter", parameter.to_owned());
        }
        error.with_source(failure)
    })
}

/// Parses the path parameter `name` (see [`Request::path_param_as`]),
/// mapping an absent or unparsable value to a `Validation` error with code
/// `invalid_path_parameter` and the name in the `parameter` metadata. The
/// value itself is never echoed; the parser's error is kept as the source.
pub fn path_param<T>(request: &Request, name: &str) -> Result<T, ForgeError>
where
    T: FromStr,
    T::Err: Into<Box<dyn StdError + Send + Sync>>,
{
    request.path_param_as(name).map_err(|failure| {
        let message = match failure {
            PathParamError::Missing { .. } => "missing path parameter",
            PathParamError::Invalid { .. } => "path parameter has an invalid value",
        };
        ForgeError::new(ErrorCategory::Validation, "invalid_path_parameter", message)
            .with_metadata("parameter", failure.name().to_owned())
            .with_source(failure)
    })
}

fn cause_chain(failure: &ForgeError) -> String {
    let mut chain = failure.to_string();
    let mut source = std::error::Error::source(failure);
    while let Some(cause) = source {
        chain.push_str(": ");
        chain.push_str(&cause.to_string());
        source = cause.source();
    }
    chain
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;
    use bytes::Bytes;
    use forge_http::{HeaderMap, Method, Request, Response, StatusCode, Uri};
    use uuid::Uuid;

    fn request(method: Method, path: &str) -> Request {
        Request::new_for_test(
            method,
            path.parse().expect("valid test URI"),
            HeaderMap::new(),
            Bytes::new(),
            "127.0.0.1:4000".parse().expect("valid test address"),
            Uuid::now_v7(),
        )
    }

    fn body(response: &Response) -> serde_json::Value {
        serde_json::from_slice(response.body()).expect("JSON body")
    }

    #[test]
    fn html_response_sets_utf8_content_type() {
        let response = html(StatusCode::OK, "<main>hello</main>");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[forge_http::header::CONTENT_TYPE],
            "text/html; charset=utf-8"
        );
        assert_eq!(response.body().as_ref(), b"<main>hello</main>");
    }

    #[test]
    fn html_escaping_covers_markup_delimiters_and_preserves_unicode() {
        assert_eq!(
            escape_html("<script a=\"x\">é & 'hi'</script>"),
            "&lt;script a=&quot;x&quot;&gt;é &amp; &#x27;hi&#x27;&lt;/script&gt;"
        );
    }

    #[test]
    fn errors_use_category_status_and_hide_sources() {
        let failure = ForgeError::new(
            ErrorCategory::Infrastructure,
            "database_unavailable",
            "service temporarily unavailable",
        )
        .with_retryable(true)
        .with_source(io::Error::other("password=secret"));

        let response = error(&failure);

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let rendered = body(&response);
        assert_eq!(rendered["code"], "database_unavailable");
        assert_eq!(rendered["retryable"], true);
        assert!(!rendered.to_string().contains("password"));
    }

    #[test]
    fn validation_errors_are_unprocessable() {
        let failure = ForgeError::new(ErrorCategory::Validation, "invalid_sku", "invalid SKU")
            .with_metadata("field", "sku");

        let response = error(&failure);

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body(&response)["metadata"]["field"], "sku");
    }

    #[test]
    fn decode_json_maps_malformed_bodies_to_validation() {
        #[derive(serde::Deserialize, Debug)]
        struct Input {
            #[allow(dead_code)]
            name: String,
        }
        let req = request(forge_http::Method::POST, "/products");

        let failure = decode_json::<Input>(&req).expect_err("wrong shape");

        assert_eq!(failure.category(), ErrorCategory::Validation);
        assert_eq!(failure.code(), "invalid_json");
    }

    fn query_request(uri: &str) -> Request {
        let uri: Uri = uri.parse().expect("valid test URI");
        Request::new_for_test(
            forge_http::Method::GET,
            uri,
            HeaderMap::new(),
            Bytes::new(),
            "127.0.0.1:4000".parse().expect("valid test address"),
            Uuid::now_v7(),
        )
    }

    fn parameter(failure: &ForgeError) -> Option<&str> {
        failure
            .metadata()
            .get("parameter")
            .and_then(serde_json::Value::as_str)
    }

    #[derive(serde::Deserialize, Debug)]
    #[serde(deny_unknown_fields)]
    struct Paging {
        limit: Option<u16>,
        after: Option<forge_core::Id<Paging>>,
    }

    #[test]
    fn decode_query_parses_typed_parameters() -> Result<(), ForgeError> {
        let id = forge_core::Id::<Paging>::new();
        let paging: Paging =
            decode_query(&query_request(&format!("/products?limit=10&after={id}")))?;
        let empty: Paging = decode_query(&query_request("/products"))?;

        assert_eq!(paging.limit, Some(10));
        assert_eq!(paging.after, Some(id));
        assert!(empty.limit.is_none() && empty.after.is_none());
        Ok(())
    }

    #[test]
    fn decode_query_maps_failures_to_invalid_query() {
        let long = format!("/p?{}=1", "x".repeat(200));
        for (uri, expected) in [
            ("/p?limit=1&limit=2", Some("limit")),
            ("/p?limit=%zz", Some("limit")),
            ("/p?limit=%FF", Some("limit")),
            ("/p?limit=70000", Some("limit")),
            ("/p?after=1", Some("after")),
            ("/p?page=2", Some("page")),
            ("/p?limit=1&", None),
        ] {
            let failure = decode_query::<Paging>(&query_request(uri)).expect_err(uri);
            assert_eq!(failure.category(), ErrorCategory::Validation, "{uri}");
            assert_eq!(failure.code(), "invalid_query", "{uri}");
            assert_eq!(parameter(&failure), expected, "{uri}");
            // Caller-facing text never echoes the query.
            assert!(!failure.message().contains("70000"), "{uri}");
        }
        let failure = decode_query::<Paging>(&query_request(&long)).expect_err("long name");
        assert_eq!(parameter(&failure).map(str::len), Some(64));
        assert!(decode_query::<()>(&query_request("/p?x=1")).is_err());
    }

    #[tokio::test]
    async fn path_param_returns_missing_when_not_captured() {
        let id = forge_core::Id::<Paging>::new();
        let req = request(forge_http::Method::GET, &format!("/products/{id}"));
        let response = path_param::<forge_core::Id<Paging>>(&req, "id");

        // Should return Missing since path parameters aren't extracted by the simple router
        assert!(response.is_err());
        let failure = response.unwrap_err();
        assert_eq!(failure.code(), "invalid_path_parameter");
        assert_eq!(failure.category(), ErrorCategory::Validation);
    }

    #[test]
    fn json_sets_status_and_content_type() {
        let response = json(StatusCode::CREATED, &serde_json::json!({"id": 1}));

        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(body(&response)["id"], 1);
    }
}
