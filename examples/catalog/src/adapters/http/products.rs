//! `/products` endpoints.
//!
//! Each handler follows the same three steps: translate the HTTP request
//! into a command or query (JSON body via [`web::decode_json`], identifiers
//! and paging from the query string), call one use case, and render the
//! result with [`web::json`] or the error with [`web::error`].
//!
//! The Forge router matches exact paths only, so the product identifier is a
//! query parameter (`/products?id=<uuid>`) rather than a path segment.

use chrono::{DateTime, Utc};
use forge::{
    core::{ErrorCategory, ForgeError, Id, Page, PageLimit, PageRequest, Version},
    http::{HeaderValue, Request, Response, StatusCode, header},
    web,
};
use serde::{Deserialize, Serialize};

use crate::{
    adapters::http::query,
    application::{
        commands::{ChangePrice, CreateProduct, RenameProduct},
        ports::ProductRepository,
        queries::{GetProduct, ListProducts},
        use_cases::ProductCatalog,
    },
    domain::{
        entities::Product,
        value_objects::{Currency, Money, ProductName, Sku},
    },
};

/// `POST /products` with a [`CreateProductBody`]: `201 Created` with the
/// product and a `Location: /products?id=<uuid>` header.
pub async fn create_product<R: ProductRepository>(
    catalog: &ProductCatalog<R>,
    request: Request,
) -> Response {
    let created = async {
        query::parse(request.uri().query(), &[])?;
        let body: CreateProductBody = web::decode_json(&request)?;
        let command = CreateProduct {
            sku: Sku::parse(&body.sku)?,
            name: ProductName::parse(&body.name)?,
            price: body.price.into_money()?,
        };
        catalog.create(command).await
    };
    match created.await {
        Ok(product) => {
            let mut response = web::json(StatusCode::CREATED, &ProductResponse::from(&product));
            // A hyphenated UUID is always a valid header value.
            if let Ok(location) = HeaderValue::try_from(format!("/products?id={}", product.id())) {
                response.headers_mut().insert(header::LOCATION, location);
            }
            response
        }
        Err(failure) => web::error(&failure),
    }
}

/// `GET /products?id=<uuid>`: one product, or `GET
/// /products[?limit=1..100][&after=<uuid>]`: one page of products, newest
/// first, as `{"items":[...],"next_cursor":"<uuid>"|null}`.
pub async fn get_products<R: ProductRepository>(
    catalog: &ProductCatalog<R>,
    request: Request,
) -> Response {
    let lookup = match ProductLookup::parse(request.uri().query()) {
        Ok(lookup) => lookup,
        Err(failure) => return web::error(&failure),
    };
    match lookup {
        ProductLookup::One(query) => match catalog.get(query).await {
            Ok(product) => web::json(StatusCode::OK, &ProductResponse::from(&product)),
            Err(failure) => web::error(&failure),
        },
        ProductLookup::Page(query) => match catalog.list(query).await {
            Ok(page) => web::json(StatusCode::OK, &page_response(&page)),
            Err(failure) => web::error(&failure),
        },
    }
}

/// `PATCH /products?id=<uuid>` with a [`PatchProductBody`] holding exactly
/// one change: `200 OK` with the updated product (its version advanced).
pub async fn patch_product<R: ProductRepository>(
    catalog: &ProductCatalog<R>,
    request: Request,
) -> Response {
    let updated = async {
        let parameters = query::parse(request.uri().query(), &["id"])?;
        let id = required_id(parameters.get("id").copied(), "id")?;
        let body: PatchProductBody = web::decode_json(&request)?;
        let expected_version = Version::new(body.expected_version)
            .map_err(|error| error.with_metadata("field", "expected_version"))?;
        match (body.name, body.price) {
            (Some(name), None) => {
                let command = RenameProduct {
                    id,
                    expected_version,
                    name: ProductName::parse(&name)?,
                };
                catalog.rename(command).await
            }
            (None, Some(price)) => {
                let command = ChangePrice {
                    id,
                    expected_version,
                    price: price.into_money()?,
                };
                catalog.change_price(command).await
            }
            _ => Err(ForgeError::new(
                ErrorCategory::Validation,
                "invalid_patch",
                "provide exactly one of `name` or `price`",
            )),
        }
    };
    match updated.await {
        Ok(product) => web::json(StatusCode::OK, &ProductResponse::from(&product)),
        Err(failure) => web::error(&failure),
    }
}

/// Body of `POST /products`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateProductBody {
    /// Stock keeping unit, e.g. `"MUG-RED"`.
    pub sku: String,
    /// Display name.
    pub name: String,
    /// Initial price.
    pub price: PriceBody,
}

/// Body of `PATCH /products?id=<uuid>`; exactly one of `name` and `price`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchProductBody {
    /// Version of the product the client read.
    pub expected_version: i64,
    /// New name.
    #[serde(default)]
    pub name: Option<String>,
    /// New price, in the product's currency.
    #[serde(default)]
    pub price: Option<PriceBody>,
}

/// A price on the wire: `{"cents": 1299, "currency": "EUR"}`.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PriceBody {
    /// Amount in minor units.
    pub cents: i64,
    /// Three-letter currency code.
    pub currency: String,
}

impl PriceBody {
    fn into_money(self) -> Result<Money, ForgeError> {
        Ok(Money::new(self.cents, Currency::parse(&self.currency)?)?)
    }
}

/// A product on the wire.
#[derive(Debug, Serialize)]
pub struct ProductResponse {
    /// UUIDv7 identifier.
    pub id: Id<Product>,
    /// Normalized SKU.
    pub sku: String,
    /// Display name.
    pub name: String,
    /// Current price.
    pub price: PriceBody,
    /// Version to send back as `expected_version` when changing the product.
    pub version: Version,
    /// When the product was first stored (informational).
    pub created_at: DateTime<Utc>,
    /// When the product was last stored (informational).
    pub updated_at: DateTime<Utc>,
}

impl From<&Product> for ProductResponse {
    fn from(product: &Product) -> Self {
        Self {
            id: product.id(),
            sku: product.sku().as_str().to_owned(),
            name: product.name().as_str().to_owned(),
            price: PriceBody {
                cents: product.price().cents(),
                currency: product.price().currency().as_str().to_owned(),
            },
            version: product.version(),
            created_at: product.created_at(),
            updated_at: product.updated_at(),
        }
    }
}

fn page_response(page: &Page<Product, Product>) -> Page<Product, ProductResponse> {
    Page {
        items: page.items.iter().map(ProductResponse::from).collect(),
        next_cursor: page.next_cursor,
    }
}

/// What a `GET /products` request asks for.
#[derive(Debug)]
enum ProductLookup {
    One(GetProduct),
    Page(ListProducts),
}

impl ProductLookup {
    fn parse(query: Option<&str>) -> Result<Self, ForgeError> {
        let parameters = query::parse(query, &["id", "limit", "after"])?;
        if let Some(id) = parameters.get("id").copied() {
            if parameters.len() > 1 {
                return Err(ForgeError::new(
                    ErrorCategory::Validation,
                    "invalid_query",
                    "`id` cannot be combined with `limit` or `after`",
                )
                .with_metadata("parameter", "id"));
            }
            return Ok(Self::One(GetProduct {
                id: required_id(Some(id), "id")?,
            }));
        }
        let limit = match parameters.get("limit") {
            Some(limit) => parse_limit(limit)?,
            None => PageLimit::DEFAULT,
        };
        let after = match parameters.get("after").copied() {
            Some(after) => Some(required_id(Some(after), "after")?),
            None => None,
        };
        Ok(Self::Page(ListProducts {
            page: PageRequest { limit, after },
        }))
    }
}

fn parse_limit(value: &str) -> Result<PageLimit, ForgeError> {
    // Out-of-range and non-numeric values get the same error as 0 or 101.
    PageLimit::new(value.parse().unwrap_or(0))
        .map_err(|error| error.with_metadata("parameter", "limit"))
}

/// Parses a product identifier from the query parameter `name`.
fn required_id(value: Option<&str>, name: &'static str) -> Result<Id<Product>, ForgeError> {
    let Some(value) = value else {
        return Err(ForgeError::new(
            ErrorCategory::Validation,
            "invalid_query",
            "missing required query parameter",
        )
        .with_metadata("parameter", name));
    };
    value.parse().map_err(|error| {
        ForgeError::new(
            ErrorCategory::Validation,
            "invalid_id",
            "identifier must be a UUIDv7",
        )
        .with_metadata("parameter", name)
        .with_source(error)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup_error(query: &str) -> Option<String> {
        ProductLookup::parse(Some(query))
            .err()
            .map(|error| error.code().to_owned())
    }

    #[test]
    fn lookup_defaults_to_the_first_page() -> Result<(), ForgeError> {
        let lookup = ProductLookup::parse(None)?;

        assert!(matches!(
            lookup,
            ProductLookup::Page(ListProducts { page }) if page.limit == PageLimit::DEFAULT && page.after.is_none()
        ));
        Ok(())
    }

    #[test]
    fn lookup_reads_id_limit_and_cursor() -> Result<(), ForgeError> {
        let id = Id::<Product>::new();

        let one = ProductLookup::parse(Some(&format!("id={id}")))?;
        assert!(matches!(one, ProductLookup::One(GetProduct { id: found }) if found == id));

        let page = ProductLookup::parse(Some(&format!("after={id}&limit=7")))?;
        assert!(matches!(
            page,
            ProductLookup::Page(ListProducts { page }) if page.limit.get() == 7 && page.after == Some(id)
        ));
        Ok(())
    }

    #[test]
    fn lookup_rejects_invalid_parameters() {
        let id = Id::<Product>::new();
        for (query, code) in [
            (format!("id={id}&limit=5"), "invalid_query"),
            ("id=not-a-uuid".to_owned(), "invalid_id"),
            // A valid UUID, but version 4.
            (
                "id=67e55044-10b1-426f-9247-bb680e5fe0c8".to_owned(),
                "invalid_id",
            ),
            ("after=1".to_owned(), "invalid_id"),
            ("limit=0".to_owned(), "invalid_page_limit"),
            ("limit=101".to_owned(), "invalid_page_limit"),
            ("limit=-1".to_owned(), "invalid_page_limit"),
            ("limit=99999999999".to_owned(), "invalid_page_limit"),
            ("limit=ten".to_owned(), "invalid_page_limit"),
            ("page=2".to_owned(), "invalid_query"),
        ] {
            assert_eq!(lookup_error(&query).as_deref(), Some(code), "{query}");
        }
    }

    #[test]
    fn price_body_is_validated_by_the_domain() {
        let cases = [
            (-1, "EUR", "invalid_price"),
            (100, "EURO", "invalid_currency"),
        ];
        for (cents, currency, code) in cases {
            let body = PriceBody {
                cents,
                currency: currency.to_owned(),
            };
            let error = body.into_money().err();
            assert_eq!(error.as_ref().map(ForgeError::code), Some(code));
        }
    }
}
