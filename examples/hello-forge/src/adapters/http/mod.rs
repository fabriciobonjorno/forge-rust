//! HTTP adapter. Handlers stay thin: decode the request, call a use case and
//! map the result to a response.

use forge::http::{Method, Request, Response, RouteError, Router, StatusCode};
use serde::Serialize;

/// Registers the application's routes. Forge adds `/health`, `/health/live`
/// and `/health/ready` itself.
pub fn routes(router: &mut Router) -> Result<(), RouteError> {
    router.route(Method::GET, "/", index)?;
    router.route(Method::GET, "/hello/{name}", hello)?;
    include!("../views/routes.rs");
    Ok(())
}

#[derive(Serialize)]
struct Index {
    application: &'static str,
    version: &'static str,
}

async fn index(_request: Request) -> Response {
    let body = Index {
        application: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
    };
    Response::json(StatusCode::OK, &body)
        .unwrap_or_else(|_| Response::empty(StatusCode::INTERNAL_SERVER_ERROR))
}

#[derive(Serialize)]
struct Greeting {
    message: String,
}

async fn hello(request: Request) -> Response {
    let name: String = match forge::web::path_param(&request, "name") {
        Ok(name) => name,
        Err(failure) => return forge::web::error(&failure),
    };
    forge::web::json(
        StatusCode::OK,
        &Greeting {
            message: format!("Hello, {name}!"),
        },
    )
}
