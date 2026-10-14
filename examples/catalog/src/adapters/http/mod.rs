//! HTTP adapter. Handlers stay thin: decode the request, call a use case and
//! map the result to a response.

pub mod products;
mod query;

use std::sync::Arc;

use forge::http::{Method, Request, Response, RouteError, Router, StatusCode};
use serde::Serialize;

use crate::application::{ports::ProductRepository, use_cases::ProductCatalog};

/// Registers the application's routes. Forge adds `/health`, `/health/live`
/// and `/health/ready` itself.
///
/// `catalog` is built once by the composition root; every route holds a
/// shared reference to it.
pub fn routes<R>(router: &mut Router, catalog: Arc<ProductCatalog<R>>) -> Result<(), RouteError>
where
    R: ProductRepository + 'static,
{
    router.route(Method::GET, "/", index)?;

    let create = Arc::clone(&catalog);
    router.route(Method::POST, "/products", move |request| {
        let catalog = Arc::clone(&create);
        async move { products::create_product(&catalog, request).await }
    })?;

    let get = Arc::clone(&catalog);
    router.route(Method::GET, "/products", move |request| {
        let catalog = Arc::clone(&get);
        async move { products::get_products(&catalog, request).await }
    })?;

    router.route(Method::PATCH, "/products", move |request| {
        let catalog = Arc::clone(&catalog);
        async move { products::patch_product(&catalog, request).await }
    })?;
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
