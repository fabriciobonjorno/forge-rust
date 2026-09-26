//! HTTP transport for Forge applications.
//!
//! The public API is owned by Forge. Hyper provides the wire protocol engine,
//! but handlers receive framework request and response types rather than Hyper
//! internals. This keeps the adapter replaceable and application code stable.

use std::{
    collections::HashMap, convert::Infallible, fmt, future::Future, net::SocketAddr, pin::Pin,
    sync::Arc, time::Duration,
};

use bytes::Bytes;
use forge_config::ServerConfig;
pub use http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use hyper::{
    body::{Body as _, Incoming},
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;
use tokio::{
    net::TcpListener,
    sync::{OwnedSemaphorePermit, Semaphore},
    task::{JoinError, JoinSet},
};
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, info_span, warn};
use uuid::Uuid;

/// Pause after a failed `accept` so transient errors such as `EMFILE` do not
/// turn the accept loop into a busy loop.
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(50);
const JSON_CONTENT_TYPE: &str = "application/json";

type BoxHandlerFuture = Pin<Box<dyn Future<Output = Response> + Send + 'static>>;

/// An HTTP request detached from the underlying protocol implementation.
#[derive(Debug)]
pub struct Request {
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
    remote_addr: SocketAddr,
    request_id: Uuid,
}

impl Request {
    /// HTTP method.
    #[must_use]
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// Request URI.
    #[must_use]
    pub fn uri(&self) -> &Uri {
        &self.uri
    }

    /// Request headers.
    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Buffered body, after the configured size limit was enforced.
    #[must_use]
    pub fn body(&self) -> &Bytes {
        &self.body
    }

    /// Peer address observed by the TCP listener.
    #[must_use]
    pub fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }

    /// Server-generated UUIDv7 correlation identifier.
    #[must_use]
    pub fn request_id(&self) -> Uuid {
        self.request_id
    }

    /// Decodes a JSON request body.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, JsonError> {
        serde_json::from_slice(&self.body).map_err(JsonError::Decode)
    }
}

/// Framework-owned HTTP response.
#[derive(Debug)]
pub struct Response {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

impl Response {
    /// Creates a response with a status and raw body.
    #[must_use]
    pub fn new(status: StatusCode, body: impl Into<Bytes>) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body: body.into(),
        }
    }

    /// Creates a JSON response without exposing serialization errors.
    pub fn json<T: Serialize>(status: StatusCode, value: &T) -> Result<Self, JsonError> {
        let body = serde_json::to_vec(value).map_err(JsonError::Encode)?;
        let mut response = Self::new(status, body);
        response.headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        Ok(response)
    }

    /// Creates an empty response.
    #[must_use]
    pub fn empty(status: StatusCode) -> Self {
        Self::new(status, Bytes::new())
    }

    /// Mutably accesses headers for adapter-specific response metadata.
    #[must_use]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }
}

/// JSON boundary error.
#[derive(Debug, Error)]
pub enum JsonError {
    /// Request JSON did not match the requested type.
    #[error("request body is not valid JSON")]
    Decode(#[source] serde_json::Error),
    /// Response serialization failed.
    #[error("response could not be serialized")]
    Encode(#[source] serde_json::Error),
}

/// Object-safe asynchronous HTTP handler.
pub trait Handler: Send + Sync + 'static {
    /// Handles one already-validated transport request.
    fn call(&self, request: Request) -> BoxHandlerFuture;
}

impl<F, Fut> Handler for F
where
    F: Fn(Request) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Response> + Send + 'static,
{
    fn call(&self, request: Request) -> BoxHandlerFuture {
        Box::pin((self)(request))
    }
}

/// Immutable route table once attached to a server.
#[derive(Default)]
pub struct Router {
    routes: HashMap<(Method, String), Arc<dyn Handler>>,
}

impl fmt::Debug for Router {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut routes: Vec<String> = self
            .routes
            .keys()
            .map(|(method, path)| format!("{method} {path}"))
            .collect();
        routes.sort_unstable();
        formatter
            .debug_struct("Router")
            .field("routes", &routes)
            .finish()
    }
}

impl Router {
    /// Creates an empty route table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a route and rejects duplicates.
    pub fn route(
        &mut self,
        method: Method,
        path: impl Into<String>,
        handler: impl Handler,
    ) -> Result<&mut Self, RouteError> {
        let path = validate_path(path.into())?;
        let key = (method.clone(), path.clone());
        if self.routes.contains_key(&key) {
            return Err(RouteError::Duplicate { method, path });
        }
        self.routes.insert(key, Arc::new(handler));
        Ok(self)
    }

    /// Registers the standard liveness and readiness probes.
    ///
    /// `GET /health`, `GET /health/live` and `GET /health/ready` answer `200`
    /// with `application/json` bodies `{"status":"ok"}`, `{"status":"live"}`
    /// and `{"status":"ready"}` respectively.
    pub fn standard_health_routes(&mut self) -> Result<&mut Self, RouteError> {
        self.route(Method::GET, "/health", |_| async {
            static_json(StatusCode::OK, r#"{"status":"ok"}"#)
        })?;
        self.route(Method::GET, "/health/live", |_| async {
            static_json(StatusCode::OK, r#"{"status":"live"}"#)
        })?;
        self.route(Method::GET, "/health/ready", |_| async {
            static_json(StatusCode::OK, r#"{"status":"ready"}"#)
        })
    }

    async fn dispatch(&self, request: Request) -> Response {
        let path = request.uri.path().to_owned();
        if let Some(handler) = self.routes.get(&(request.method.clone(), path.clone())) {
            return handler.call(request).await;
        }

        if self
            .routes
            .keys()
            .any(|(_, route_path)| route_path == &path)
        {
            return Response::new(StatusCode::METHOD_NOT_ALLOWED, "method not allowed");
        }

        Response::new(StatusCode::NOT_FOUND, "not found")
    }
}

fn static_json(status: StatusCode, body: &'static str) -> Response {
    let mut response = Response::new(status, body);
    response.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(JSON_CONTENT_TYPE),
    );
    response
}

fn validate_path(path: String) -> Result<String, RouteError> {
    if !path.starts_with('/') || path.contains('?') || path.contains('#') {
        return Err(RouteError::InvalidPath(path));
    }
    Ok(path)
}

/// Route registration error detected before serving traffic.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum RouteError {
    /// Paths must be absolute and must not include query or fragment syntax.
    #[error("invalid route path {0:?}")]
    InvalidPath(String),
    /// A method/path pair may be registered only once.
    #[error("duplicate route: {method} {path}")]
    Duplicate {
        /// HTTP method.
        method: Method,
        /// Absolute request path.
        path: String,
    },
}

/// HTTP server with bounded requests and owned connection tasks.
pub struct Server {
    config: ServerConfig,
    router: Arc<Router>,
}

impl Server {
    /// Creates a server from validated configuration and a frozen router.
    #[must_use]
    pub fn new(config: ServerConfig, router: Router) -> Self {
        Self {
            config,
            router: Arc::new(router),
        }
    }

    /// Binds the configured address and serves until the shutdown future resolves.
    pub async fn run(self, shutdown: impl Future<Output = ()> + Send) -> Result<(), ServerError> {
        let listener = TcpListener::bind(self.config.bind)
            .await
            .map_err(ServerError::Bind)?;
        self.serve(listener, shutdown).await
    }

    /// Serves on an existing listener, useful for socket activation and tests.
    ///
    /// At most [`ServerConfig::max_connections`] connections are open at once;
    /// while saturated the server stops accepting, leaving new clients in the
    /// kernel backlog. Transient `accept` failures are logged and retried.
    /// When `shutdown` resolves the server stops accepting, asks open
    /// connections to finish their in-flight request, and aborts whatever is
    /// still running after [`ServerConfig::shutdown_grace`].
    pub async fn serve(
        self,
        listener: TcpListener,
        shutdown: impl Future<Output = ()> + Send,
    ) -> Result<(), ServerError> {
        let cancellation = CancellationToken::new();
        let mut connections = JoinSet::new();
        let permits = Arc::new(Semaphore::new(
            self.config.max_connections.clamp(1, Semaphore::MAX_PERMITS),
        ));
        tokio::pin!(shutdown);

        'accept: loop {
            // Admission control happens before `accept`: while every permit is
            // held, pending clients wait in the listen backlog.
            let permit = tokio::select! {
                biased;
                () = &mut shutdown => break 'accept,
                Some(completed) = connections.join_next(), if !connections.is_empty() => {
                    log_connection_result(completed);
                    continue 'accept;
                }
                permit = Arc::clone(&permits).acquire_owned() => match permit {
                    Ok(permit) => permit,
                    // The semaphore is owned by this function and never closed.
                    Err(_closed) => break 'accept,
                },
            };

            loop {
                tokio::select! {
                    biased;
                    () = &mut shutdown => break 'accept,
                    Some(completed) = connections.join_next(), if !connections.is_empty() => {
                        log_connection_result(completed);
                    }
                    accepted = listener.accept() => match accepted {
                        Ok((stream, remote_addr)) => {
                            self.spawn_connection(
                                &mut connections,
                                stream,
                                remote_addr,
                                permit,
                                cancellation.child_token(),
                            );
                            continue 'accept;
                        }
                        Err(error) => {
                            warn!(%error, "failed to accept HTTP connection; retrying");
                            tokio::select! {
                                biased;
                                () = &mut shutdown => break 'accept,
                                () = tokio::time::sleep(ACCEPT_ERROR_BACKOFF) => {}
                            }
                        }
                    },
                }
            }
        }

        drop(listener);
        cancellation.cancel();
        let grace = tokio::time::sleep(self.config.shutdown_grace);
        tokio::pin!(grace);
        // Drain until every connection has finished or the grace period ends.
        while !connections.is_empty() {
            tokio::select! {
                () = &mut grace => {
                    warn!(
                        remaining = connections.len(),
                        "shutdown grace period elapsed; aborting open connections"
                    );
                    connections.abort_all();
                    break;
                }
                Some(completed) = connections.join_next() => {
                    if let Err(error) = completed {
                        warn!(%error, "connection task failed during shutdown");
                    }
                }
            }
        }

        while connections.join_next().await.is_some() {}
        Ok(())
    }

    fn spawn_connection(
        &self,
        connections: &mut JoinSet<()>,
        stream: tokio::net::TcpStream,
        remote_addr: SocketAddr,
        permit: OwnedSemaphorePermit,
        cancellation: CancellationToken,
    ) {
        let router = Arc::clone(&self.router);
        let max_body_bytes = self.config.max_body_bytes;
        let request_timeout = self.config.request_timeout;
        connections.spawn(async move {
            serve_connection(
                stream,
                remote_addr,
                router,
                max_body_bytes,
                request_timeout,
                cancellation,
            )
            .await;
            // The connection slot is released only once the connection ends.
            drop(permit);
        });
    }
}

fn log_connection_result(completed: Result<(), JoinError>) {
    if let Err(error) = completed {
        warn!(%error, "connection task failed");
    }
}

async fn serve_connection(
    stream: tokio::net::TcpStream,
    remote_addr: SocketAddr,
    router: Arc<Router>,
    max_body_bytes: usize,
    request_timeout: Duration,
    cancellation: CancellationToken,
) {
    let service = service_fn(move |request| {
        handle_request(
            request,
            remote_addr,
            Arc::clone(&router),
            max_body_bytes,
            request_timeout,
        )
    });
    let connection = hyper::server::conn::http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(request_timeout)
        .serve_connection(TokioIo::new(stream), service);
    tokio::pin!(connection);

    tokio::select! {
        result = connection.as_mut() => {
            if let Err(error) = result {
                warn!(%error, %remote_addr, "HTTP connection failed");
            }
        }
        () = cancellation.cancelled() => {
            connection.as_mut().graceful_shutdown();
            if let Err(error) = connection.await {
                warn!(%error, %remote_addr, "HTTP connection failed during shutdown");
            }
        }
    }
}

async fn handle_request(
    request: hyper::Request<Incoming>,
    remote_addr: SocketAddr,
    router: Arc<Router>,
    max_body_bytes: usize,
    request_timeout: Duration,
) -> Result<hyper::Response<Full<Bytes>>, Infallible> {
    let request_id = Uuid::now_v7();
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let span = info_span!("http.request", %request_id, %method, %path, %remote_addr);
    let response = async move {
        let (parts, body) = request.into_parts();
        // The deadline covers reading the body as well as the handler, so a
        // client trickling body bytes cannot hold a connection indefinitely.
        let exchange = async {
            let body = match collect_body(body, max_body_bytes).await {
                Ok(body) => body,
                Err(rejection) => return rejection.into_response(),
            };
            let request = Request {
                method: parts.method,
                uri: parts.uri,
                headers: parts.headers,
                body,
                remote_addr,
                request_id,
            };
            router.dispatch(request).await
        };
        let response = match tokio::time::timeout(request_timeout, exchange).await {
            Ok(response) => response,
            Err(_elapsed) => Response::new(StatusCode::GATEWAY_TIMEOUT, "request timed out"),
        };
        to_hyper_response(response, request_id)
    }
    .instrument(span)
    .await;

    Ok(response)
}

/// Reason a request body was rejected before dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BodyRejection {
    /// The body exceeds `max_body_bytes`.
    TooLarge,
    /// The body stream failed, e.g. malformed chunked encoding.
    Unreadable,
}

impl BodyRejection {
    fn into_response(self) -> Response {
        match self {
            Self::TooLarge => {
                Response::new(StatusCode::PAYLOAD_TOO_LARGE, "request body too large")
            }
            Self::Unreadable => {
                Response::new(StatusCode::BAD_REQUEST, "request body could not be read")
            }
        }
    }
}

/// Buffers a request body, enforcing the configured size limit.
async fn collect_body(body: Incoming, max_body_bytes: usize) -> Result<Bytes, BodyRejection> {
    // Reject a declared Content-Length above the limit without reading it.
    let limit = u64::try_from(max_body_bytes).unwrap_or(u64::MAX);
    if body.size_hint().lower() > limit {
        return Err(BodyRejection::TooLarge);
    }
    match Limited::new(body, max_body_bytes).collect().await {
        Ok(collected) => Ok(collected.to_bytes()),
        Err(error) if error.is::<LengthLimitError>() => Err(BodyRejection::TooLarge),
        Err(_) => Err(BodyRejection::Unreadable),
    }
}

fn to_hyper_response(response: Response, request_id: Uuid) -> hyper::Response<Full<Bytes>> {
    let mut outgoing = hyper::Response::new(Full::new(response.body));
    *outgoing.status_mut() = response.status;
    *outgoing.headers_mut() = response.headers;
    let request_id = HeaderValue::from_str(&request_id.to_string())
        .unwrap_or_else(|error| unreachable!("UUID is always a valid header value: {error}"));
    outgoing.headers_mut().insert("x-request-id", request_id);
    outgoing.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    outgoing
        .headers_mut()
        .insert("x-frame-options", HeaderValue::from_static("DENY"));
    outgoing
}

/// Server startup failure.
///
/// Failures to accept individual connections are transient and are logged
/// and retried rather than reported here.
#[derive(Debug, Error)]
pub enum ServerError {
    /// The configured socket could not be bound.
    #[error("failed to bind HTTP listener")]
    Bind(#[source] std::io::Error),
}

#[cfg(test)]
mod tests {
    use std::{
        io::{ErrorKind, Read, Write},
        net::TcpStream as StdTcpStream,
    };

    use tokio::{sync::oneshot, task::JoinHandle};

    use super::*;

    /// Upper bound for any single client wait; generous to avoid CI flakiness.
    const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

    fn request(method: Method, path: &str) -> Request {
        Request {
            method,
            uri: path.parse().expect("valid test URI"),
            headers: HeaderMap::new(),
            body: Bytes::new(),
            remote_addr: "127.0.0.1:4000".parse().expect("valid test address"),
            request_id: Uuid::now_v7(),
        }
    }

    #[tokio::test]
    async fn routes_to_registered_handler() {
        let mut router = Router::new();
        router
            .route(Method::GET, "/customers", |_| async {
                Response::new(StatusCode::OK, "customers")
            })
            .expect("route must register");

        let response = router.dispatch(request(Method::GET, "/customers")).await;

        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, "customers");
    }

    #[tokio::test]
    async fn distinguishes_unknown_path_from_wrong_method() {
        let mut router = Router::new();
        router
            .route(Method::GET, "/customers", |_| async {
                Response::empty(StatusCode::OK)
            })
            .expect("route must register");

        let wrong_method = router.dispatch(request(Method::POST, "/customers")).await;
        let missing = router.dispatch(request(Method::GET, "/missing")).await;

        assert_eq!(wrong_method.status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(missing.status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn duplicate_routes_are_rejected() {
        let mut router = Router::new();
        router
            .route(Method::GET, "/health", |_| async {
                Response::empty(StatusCode::OK)
            })
            .expect("first route must register");
        let error = router
            .route(Method::GET, "/health", |_| async {
                Response::empty(StatusCode::OK)
            })
            .expect_err("duplicate route must fail");

        assert!(matches!(error, RouteError::Duplicate { .. }));
    }

    #[test]
    fn security_headers_are_applied_at_transport_boundary() {
        let outgoing = to_hyper_response(Response::empty(StatusCode::NO_CONTENT), Uuid::now_v7());

        assert_eq!(outgoing.headers()["x-content-type-options"], "nosniff");
        assert_eq!(outgoing.headers()["x-frame-options"], "DENY");
        assert!(outgoing.headers().contains_key("x-request-id"));
    }
    #[test]
    fn router_debug_lists_routes() {
        let mut router = Router::new();
        router
            .standard_health_routes()
            .expect("health routes must register");

        let debug = format!("{router:?}");

        assert!(debug.contains("GET /health/live"), "{debug}");
    }

    #[tokio::test]
    async fn health_routes_return_json() {
        let mut router = Router::new();
        router
            .standard_health_routes()
            .expect("health routes must register");

        for (path, body) in [
            ("/health", r#"{"status":"ok"}"#),
            ("/health/live", r#"{"status":"live"}"#),
            ("/health/ready", r#"{"status":"ready"}"#),
        ] {
            let response = router.dispatch(request(Method::GET, path)).await;

            assert_eq!(response.status, StatusCode::OK, "{path}");
            assert_eq!(response.body, body, "{path}");
            assert_eq!(
                response.headers[header::CONTENT_TYPE],
                "application/json",
                "{path}"
            );
        }
    }

    struct TestServer {
        addr: SocketAddr,
        shutdown: oneshot::Sender<()>,
        handle: JoinHandle<Result<(), ServerError>>,
    }

    impl TestServer {
        async fn start(config: ServerConfig, router: Router) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("test listener must bind");
            let addr = listener.local_addr().expect("listener has an address");
            let (shutdown, signal) = oneshot::channel::<()>();
            let server = Server::new(config, router);
            let handle = tokio::spawn(server.serve(listener, async {
                // A dropped sender also means shutdown.
                let _ = signal.await;
            }));
            Self {
                addr,
                shutdown,
                handle,
            }
        }

        async fn stop(self) {
            self.shutdown
                .send(())
                .expect("server must still be waiting for shutdown");
            tokio::time::timeout(CLIENT_TIMEOUT, self.handle)
                .await
                .expect("server must stop promptly")
                .expect("server task must not panic")
                .expect("server must shut down cleanly");
        }
    }

    fn test_config(request_timeout: Duration) -> ServerConfig {
        ServerConfig {
            bind: "127.0.0.1:0".parse().expect("valid address"),
            request_timeout,
            shutdown_grace: Duration::from_secs(30),
            max_body_bytes: 64,
            max_connections: 16,
        }
    }

    fn test_router() -> Router {
        let mut router = Router::new();
        router
            .standard_health_routes()
            .expect("health routes must register");
        router
            .route(Method::POST, "/echo", |request: Request| async move {
                Response::new(StatusCode::OK, request.body().clone())
            })
            .expect("echo route must register");
        router
            .route(Method::GET, "/slow", |_| async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Response::empty(StatusCode::OK)
            })
            .expect("slow route must register");
        router
    }

    fn connect(addr: SocketAddr) -> StdTcpStream {
        let stream = StdTcpStream::connect(addr).expect("client must connect");
        stream
            .set_read_timeout(Some(CLIENT_TIMEOUT))
            .expect("read timeout must be settable");
        stream
            .set_write_timeout(Some(CLIENT_TIMEOUT))
            .expect("write timeout must be settable");
        stream
    }

    /// Writes raw bytes and reads until the server closes the connection.
    fn exchange_blocking(addr: SocketAddr, raw: &[u8]) -> String {
        let mut stream = connect(addr);
        stream.write_all(raw).expect("request must be written");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .expect("server must close the connection before the client timeout");
        String::from_utf8_lossy(&response).into_owned()
    }

    async fn exchange(addr: SocketAddr, raw: &'static str) -> String {
        tokio::task::spawn_blocking(move || exchange_blocking(addr, raw.as_bytes()))
            .await
            .expect("client task must not panic")
    }

    /// Reads from a keep-alive connection until `marker` has been received.
    fn read_until(stream: &mut StdTcpStream, marker: &str) -> String {
        let mut received = Vec::new();
        let mut buffer = [0_u8; 1024];
        while !String::from_utf8_lossy(&received).contains(marker) {
            let read = stream.read(&mut buffer).expect("response must arrive");
            assert!(read > 0, "connection closed before {marker:?} arrived");
            received.extend_from_slice(&buffer[..read]);
        }
        String::from_utf8_lossy(&received).into_owned()
    }

    #[tokio::test]
    async fn serve_answers_liveness_and_stops_on_shutdown() {
        let server = TestServer::start(test_config(Duration::from_secs(10)), test_router()).await;

        let response = exchange(
            server.addr,
            "GET /health/live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;

        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
        assert!(
            response
                .to_ascii_lowercase()
                .contains("content-type: application/json\r\n"),
            "{response}"
        );
        assert!(response.ends_with(r#"{"status":"live"}"#), "{response}");
        server.stop().await;
    }

    #[tokio::test]
    async fn slow_handler_times_out_with_gateway_timeout() {
        let server =
            TestServer::start(test_config(Duration::from_millis(200)), test_router()).await;

        let response = exchange(
            server.addr,
            "GET /slow HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;

        assert!(response.starts_with("HTTP/1.1 504 "), "{response}");
        server.stop().await;
    }

    #[tokio::test]
    async fn request_timeout_covers_slow_body() {
        let server =
            TestServer::start(test_config(Duration::from_millis(200)), test_router()).await;

        // Declares ten body bytes but sends two, then stalls.
        let response = exchange(
            server.addr,
            "POST /echo HTTP/1.1\r\nHost: localhost\r\nContent-Length: 10\r\n\
             Connection: close\r\n\r\nab",
        )
        .await;

        assert!(response.starts_with("HTTP/1.1 504 "), "{response}");
        server.stop().await;
    }

    #[tokio::test]
    async fn slow_headers_close_the_connection() {
        let server =
            TestServer::start(test_config(Duration::from_millis(200)), test_router()).await;

        // The header block is never terminated.
        let response = exchange(server.addr, "GET /health HTTP/1.1\r\nHost: localhost\r\n").await;

        assert!(!response.contains(" 200 "), "{response}");
        server.stop().await;
    }

    #[tokio::test]
    async fn complete_body_is_delivered_to_handler() {
        let server = TestServer::start(test_config(Duration::from_secs(10)), test_router()).await;

        let response = exchange(
            server.addr,
            "POST /echo HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\n\
             Connection: close\r\n\r\nhello",
        )
        .await;

        assert!(response.starts_with("HTTP/1.1 200 "), "{response}");
        assert!(response.ends_with("\r\n\r\nhello"), "{response}");
        server.stop().await;
    }

    #[tokio::test]
    async fn oversized_bodies_are_rejected() {
        let server = TestServer::start(test_config(Duration::from_secs(10)), test_router()).await;

        let declared = exchange(
            server.addr,
            "POST /echo HTTP/1.1\r\nHost: localhost\r\nContent-Length: 1000\r\n\
             Connection: close\r\n\r\n",
        )
        .await;
        assert!(declared.starts_with("HTTP/1.1 413 "), "{declared}");

        // An 80-byte chunk exceeds the 64-byte limit without a Content-Length.
        let chunked = exchange(
            server.addr,
            "POST /echo HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\
             Connection: close\r\n\r\n50\r\n\
             01234567890123456789012345678901234567890123456789012345678901234567890123456789\
             \r\n0\r\n\r\n",
        )
        .await;
        assert!(chunked.starts_with("HTTP/1.1 413 "), "{chunked}");
        server.stop().await;
    }

    #[tokio::test]
    async fn connection_limit_applies_backpressure() {
        let mut config = test_config(Duration::from_secs(10));
        config.max_connections = 1;
        let server = TestServer::start(config, test_router()).await;
        let addr = server.addr;

        tokio::task::spawn_blocking(move || {
            // The first client keeps its connection (and the only permit) open.
            let mut holder = connect(addr);
            holder
                .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .expect("request must be written");
            let first = read_until(&mut holder, r#"{"status":"ok"}"#);
            assert!(first.starts_with("HTTP/1.1 200 "), "{first}");

            // The second client sits in the listen backlog and gets no answer.
            let mut waiting = connect(addr);
            waiting
                .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .expect("request must be written");
            waiting
                .set_read_timeout(Some(Duration::from_millis(300)))
                .expect("read timeout must be settable");
            let mut probe = [0_u8; 1];
            let error = waiting
                .read(&mut probe)
                .expect_err("saturated server must not answer");
            assert!(
                matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut),
                "{error}"
            );

            // Releasing the first connection frees the permit.
            drop(holder);
            waiting
                .set_read_timeout(Some(CLIENT_TIMEOUT))
                .expect("read timeout must be settable");
            let mut second = Vec::new();
            waiting
                .read_to_end(&mut second)
                .expect("second client must be served once a permit is free");
            let second = String::from_utf8_lossy(&second);
            assert!(second.starts_with("HTTP/1.1 200 "), "{second}");
        })
        .await
        .expect("client task must not panic");

        server.stop().await;
    }

    #[tokio::test]
    async fn shutdown_interrupts_saturated_server_and_closes_idle_connections() {
        let mut config = test_config(Duration::from_secs(10));
        config.max_connections = 1;
        let server = TestServer::start(config, test_router()).await;
        let addr = server.addr;

        // Hold the only permit with an idle keep-alive connection.
        let holder = tokio::task::spawn_blocking(move || {
            let mut holder = connect(addr);
            holder
                .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .expect("request must be written");
            read_until(&mut holder, r#"{"status":"ok"}"#);
            holder
        })
        .await
        .expect("client task must not panic");

        // Grace is 30 s; `stop` requires completion well before that.
        server.stop().await;
        drop(holder);
    }
}
