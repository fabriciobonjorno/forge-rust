//! Shared helpers for unit tests.

use bytes::Bytes;
use forge_http::{HeaderMap, HeaderValue, Method, Request, Response, header::HeaderName};
use uuid::Uuid;

/// Builds a request with the given method, URI and headers.
pub(crate) fn request_with(method: Method, uri: &str, headers: &[(&str, &str)]) -> Request {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.insert(
            HeaderName::from_bytes(name.as_bytes()).expect("valid header name"),
            HeaderValue::from_str(value).expect("valid header value"),
        );
    }
    Request::new_for_test(
        method,
        uri.parse().expect("valid test URI"),
        map,
        Bytes::new(),
        "127.0.0.1:4000".parse().expect("valid test address"),
        Uuid::now_v7(),
    )
}

/// Builds a `GET` request for `uri`.
pub(crate) fn get(uri: &str) -> Request {
    request_with(Method::GET, uri, &[])
}

/// The response body as UTF-8 text.
pub(crate) fn text(response: &Response) -> String {
    String::from_utf8(response.body().to_vec()).expect("UTF-8 body")
}

/// A response header as text, if present.
pub(crate) fn header<'r>(response: &'r Response, name: &str) -> Option<&'r str> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}
