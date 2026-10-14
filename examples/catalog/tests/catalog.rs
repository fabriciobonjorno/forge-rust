//! Hermetic tests: the repository contract against the in-memory double, the
//! use cases, and the HTTP handlers, all without PostgreSQL.

mod support;

use std::{error::Error, sync::Arc};

use app::{
    adapters::http::{self, products},
    application::{
        commands::{ChangePrice, CreateProduct, RenameProduct},
        queries::{GetProduct, ListProducts},
        use_cases::ProductCatalog,
    },
    domain::value_objects::{Currency, Money, ProductName, Sku},
};
use forge::{
    core::{ErrorCategory, ForgeError, Id, PageLimit, PageRequest, Version},
    http::{Method, Request, Response, Router, StatusCode, Uri, header},
};
use serde_json::{Value, json};
use support::{contract, memory::InMemoryProductRepository};

type TestResult = Result<(), Box<dyn Error>>;

fn catalog() -> ProductCatalog<InMemoryProductRepository> {
    ProductCatalog::new(InMemoryProductRepository::default())
}

// Repository contract ------------------------------------------------------

#[tokio::test]
async fn memory_contract_insert_then_find() -> TestResult {
    contract::insert_then_find(&InMemoryProductRepository::default()).await
}

#[tokio::test]
async fn memory_contract_duplicate_sku_is_rejected() -> TestResult {
    contract::duplicate_sku_is_rejected(&InMemoryProductRepository::default()).await
}

#[tokio::test]
async fn memory_contract_update_advances_the_version() -> TestResult {
    contract::update_advances_the_version(&InMemoryProductRepository::default()).await
}

#[tokio::test]
async fn memory_contract_stale_update_conflicts() -> TestResult {
    contract::stale_update_conflicts(&InMemoryProductRepository::default()).await
}

#[tokio::test]
async fn memory_contract_update_of_missing_product_is_not_found() -> TestResult {
    contract::update_of_missing_product_is_not_found(&InMemoryProductRepository::default()).await
}

#[tokio::test]
async fn memory_contract_cursor_pagination_visits_every_product_once() -> TestResult {
    contract::cursor_pagination_visits_every_product_once(&InMemoryProductRepository::default())
        .await
}

// Use cases ------------------------------------------------------------------

fn create_command(sku: &str) -> Result<CreateProduct, Box<dyn Error>> {
    Ok(CreateProduct {
        sku: Sku::parse(sku)?,
        name: ProductName::parse("Mug")?,
        price: Money::new(1_299, Currency::parse("EUR")?)?,
    })
}

fn assert_code(
    result: Result<impl std::fmt::Debug, ForgeError>,
    category: ErrorCategory,
    code: &str,
) {
    match result {
        Ok(value) => panic!("expected {code}, got {value:?}"),
        Err(error) => {
            assert_eq!(error.category(), category, "{error:?}");
            assert_eq!(error.code(), code);
        }
    }
}

#[tokio::test]
async fn create_then_get_returns_the_stored_product() -> TestResult {
    let catalog = catalog();

    let created = catalog.create(create_command("mug-1")?).await?;
    let loaded = catalog.get(GetProduct { id: created.id() }).await?;

    assert_eq!(loaded, created);
    assert_eq!(loaded.sku().as_str(), "MUG-1");
    assert_eq!(loaded.version(), Version::INITIAL);
    Ok(())
}

#[tokio::test]
async fn duplicate_sku_is_a_conflict() -> TestResult {
    let catalog = catalog();
    catalog.create(create_command("MUG-1")?).await?;

    let result = catalog.create(create_command("mug-1")?).await;

    assert_code(result, ErrorCategory::Conflict, "sku_taken");
    Ok(())
}

#[tokio::test]
async fn unknown_product_is_not_found() -> TestResult {
    let catalog = catalog();
    let id = Id::new();

    assert_code(
        catalog.get(GetProduct { id }).await,
        ErrorCategory::NotFound,
        "product_not_found",
    );
    let rename = RenameProduct {
        id,
        expected_version: Version::INITIAL,
        name: ProductName::parse("Nope")?,
    };
    assert_code(
        catalog.rename(rename).await,
        ErrorCategory::NotFound,
        "product_not_found",
    );
    Ok(())
}

#[tokio::test]
async fn rename_and_change_price_advance_the_version() -> TestResult {
    let catalog = catalog();
    let created = catalog.create(create_command("MUG-1")?).await?;

    let renamed = catalog
        .rename(RenameProduct {
            id: created.id(),
            expected_version: created.version(),
            name: ProductName::parse("  Big mug ")?,
        })
        .await?;
    let repriced = catalog
        .change_price(ChangePrice {
            id: created.id(),
            expected_version: renamed.version(),
            price: Money::new(1_499, Currency::parse("eur")?)?,
        })
        .await?;

    assert_eq!(renamed.name().as_str(), "Big mug");
    assert_eq!(renamed.version().get(), 2);
    assert_eq!(repriced.price().cents(), 1_499);
    assert_eq!(repriced.name().as_str(), "Big mug");
    assert_eq!(repriced.version().get(), 3);
    Ok(())
}

#[tokio::test]
async fn stale_expected_version_is_a_conflict() -> TestResult {
    let catalog = catalog();
    let created = catalog.create(create_command("MUG-1")?).await?;
    let rename = |name: &str| -> Result<RenameProduct, Box<dyn Error>> {
        Ok(RenameProduct {
            id: created.id(),
            expected_version: created.version(),
            name: ProductName::parse(name)?,
        })
    };
    catalog.rename(rename("First")?).await?;

    let result = catalog.rename(rename("Second")?).await;

    assert_code(result, ErrorCategory::Conflict, "concurrent_modification");
    let current = catalog.get(GetProduct { id: created.id() }).await?;
    assert_eq!(current.name().as_str(), "First");
    Ok(())
}

#[tokio::test]
async fn changing_the_currency_is_rejected() -> TestResult {
    let catalog = catalog();
    let created = catalog.create(create_command("MUG-1")?).await?;

    let result = catalog
        .change_price(ChangePrice {
            id: created.id(),
            expected_version: created.version(),
            price: Money::new(1_000, Currency::parse("USD")?)?,
        })
        .await;

    assert_code(result, ErrorCategory::Validation, "currency_mismatch");
    Ok(())
}

#[tokio::test]
async fn list_pages_newest_first() -> TestResult {
    let catalog = catalog();
    let first = catalog.create(create_command("SKU-A")?).await?;
    let second = catalog.create(create_command("SKU-B")?).await?;

    let page = catalog
        .list(ListProducts {
            page: PageRequest {
                limit: PageLimit::new(1)?,
                after: None,
            },
        })
        .await?;
    let expected_newest = first.id().max(second.id());
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id(), expected_newest);
    assert_eq!(page.next_cursor, Some(expected_newest));

    let everything = catalog.list(ListProducts::default()).await?;
    assert_eq!(everything.items.len(), 2);
    assert_eq!(everything.next_cursor, None);
    Ok(())
}

// HTTP handlers ----------------------------------------------------------------

fn request(method: Method, uri: &str, body: &str) -> Result<Request, Box<dyn Error>> {
    Ok(Request::new_for_test(
        method,
        uri.parse::<Uri>()?,
        Default::default(),
        body.to_owned().into(),
        "127.0.0.1:0".parse()?,
        uuid::Uuid::now_v7(),
    ))
}

fn body(response: &Response) -> Result<Value, serde_json::Error> {
    serde_json::from_slice(response.body())
}

async fn post(
    catalog: &ProductCatalog<InMemoryProductRepository>,
    payload: &Value,
) -> Result<Response, Box<dyn Error>> {
    let request = request(Method::POST, "/products", &payload.to_string())?;
    Ok(products::create_product(catalog, request).await)
}

fn mug(sku: &str) -> Value {
    json!({"sku": sku, "name": "Mug", "price": {"cents": 1299, "currency": "eur"}})
}

#[tokio::test]
async fn post_creates_with_location_and_get_returns_it() -> TestResult {
    let catalog = catalog();

    let created = post(&catalog, &mug("mug-1")).await?;

    assert_eq!(created.status(), StatusCode::CREATED);
    let product = body(&created)?;
    assert_eq!(product["sku"], "MUG-1");
    assert_eq!(product["price"], json!({"cents": 1299, "currency": "EUR"}));
    assert_eq!(product["version"], 1);
    let id = product["id"].as_str().ok_or("id")?;
    let location = created
        .headers()
        .get(header::LOCATION)
        .ok_or("Location header")?
        .to_str()?;
    assert_eq!(location, format!("/products?id={id}"));

    let fetched = products::get_products(&catalog, request(Method::GET, location, "")?).await;
    assert_eq!(fetched.status(), StatusCode::OK);
    assert_eq!(body(&fetched)?, product);
    Ok(())
}

#[tokio::test]
async fn post_rejects_invalid_input() -> TestResult {
    let catalog = catalog();
    let cases = [
        (mug("x"), "invalid_sku", "sku"),
        (
            json!({"sku": "MUG-1", "name": " ", "price": {"cents": 1, "currency": "EUR"}}),
            "invalid_name",
            "name",
        ),
        (
            json!({"sku": "MUG-1", "name": "Mug", "price": {"cents": -1, "currency": "EUR"}}),
            "invalid_price",
            "price.cents",
        ),
        (
            json!({"sku": "MUG-1", "name": "Mug", "price": {"cents": 1, "currency": "€"}}),
            "invalid_currency",
            "price.currency",
        ),
    ];
    for (payload, code, field) in cases {
        let response = post(&catalog, &payload).await?;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{code}"
        );
        let error = body(&response)?;
        assert_eq!(error["code"], code);
        assert_eq!(error["metadata"]["field"], field);
    }
    Ok(())
}

#[tokio::test]
async fn malformed_json_and_unknown_fields_are_invalid_json() -> TestResult {
    let catalog = catalog();
    for payload in [
        "{not json",
        r#"{"sku":"MUG-1","name":"Mug"}"#,
        r#"{"sku":"MUG-1","name":"Mug","price":{"cents":1,"currency":"EUR"},"admin":true}"#,
    ] {
        let response =
            products::create_product(&catalog, request(Method::POST, "/products", payload)?).await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{payload}"
        );
        assert_eq!(body(&response)?["code"], "invalid_json", "{payload}");
    }
    Ok(())
}

#[tokio::test]
async fn duplicate_sku_is_409() -> TestResult {
    let catalog = catalog();
    post(&catalog, &mug("MUG-1")).await?;

    let response = post(&catalog, &mug("mug-1")).await?;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(body(&response)?["code"], "sku_taken");
    Ok(())
}

#[tokio::test]
async fn get_unknown_product_is_404_and_bad_queries_are_422() -> TestResult {
    let catalog = catalog();
    let unknown = format!("/products?id={}", Id::<()>::new());

    let response = products::get_products(&catalog, request(Method::GET, &unknown, "")?).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(body(&response)?["code"], "product_not_found");

    for (uri, code) in [
        ("/products?id=42", "invalid_id"),
        ("/products?limit=0", "invalid_page_limit"),
        ("/products?limit=5&limit=6", "invalid_query"),
        ("/products?sort=name", "invalid_query"),
    ] {
        let response = products::get_products(&catalog, request(Method::GET, uri, "")?).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY, "{uri}");
        assert_eq!(body(&response)?["code"], code, "{uri}");
    }
    Ok(())
}

#[tokio::test]
async fn list_follows_the_cursor() -> TestResult {
    let catalog = catalog();
    for sku in ["SKU-A", "SKU-B", "SKU-C"] {
        post(&catalog, &mug(sku)).await?;
    }

    let first =
        products::get_products(&catalog, request(Method::GET, "/products?limit=2", "")?).await;
    let first = body(&first)?;
    let cursor = first["next_cursor"].as_str().ok_or("cursor")?;
    let second = products::get_products(
        &catalog,
        request(
            Method::GET,
            &format!("/products?limit=2&after={cursor}"),
            "",
        )?,
    )
    .await;
    let second = body(&second)?;

    assert_eq!(first["items"].as_array().map(Vec::len), Some(2));
    assert_eq!(second["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(second["next_cursor"], Value::Null);
    Ok(())
}

#[tokio::test]
async fn patch_applies_one_change_with_optimistic_locking() -> TestResult {
    let catalog = catalog();
    let created = body(&post(&catalog, &mug("MUG-1")).await?)?;
    let uri = format!("/products?id={}", created["id"].as_str().ok_or("id")?);
    let patch = |payload: Value| -> Result<Request, Box<dyn Error>> {
        request(Method::PATCH, &uri, &payload.to_string())
    };

    let renamed = products::patch_product(
        &catalog,
        patch(json!({"expected_version": 1, "name": "Cup"}))?,
    )
    .await;
    assert_eq!(renamed.status(), StatusCode::OK);
    assert_eq!(body(&renamed)?["version"], 2);

    let repriced = products::patch_product(
        &catalog,
        patch(json!({"expected_version": 2, "price": {"cents": 999, "currency": "EUR"}}))?,
    )
    .await;
    let repriced = body(&repriced)?;
    assert_eq!(repriced["name"], "Cup");
    assert_eq!(repriced["price"]["cents"], 999);
    assert_eq!(repriced["version"], 3);

    let cases = [
        (
            json!({"expected_version": 1, "name": "Stale"}),
            StatusCode::CONFLICT,
            "concurrent_modification",
        ),
        (
            json!({"expected_version": 3}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_patch",
        ),
        (
            json!({"expected_version": 3, "name": "A", "price": {"cents": 1, "currency": "EUR"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_patch",
        ),
        (
            json!({"expected_version": 0, "name": "A"}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_version",
        ),
        (
            json!({"expected_version": 3, "price": {"cents": 1, "currency": "USD"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "currency_mismatch",
        ),
    ];
    for (payload, status, code) in cases {
        let response = products::patch_product(&catalog, patch(payload)?).await;
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(body(&response)?["code"], code);
    }

    let missing_id =
        products::patch_product(&catalog, request(Method::PATCH, "/products", "{}")?).await;
    assert_eq!(body(&missing_id)?["code"], "invalid_query");
    Ok(())
}

#[test]
fn routes_register_the_product_endpoints() -> TestResult {
    let mut router = Router::new();

    http::routes(&mut router, Arc::new(catalog()))?;

    let registered = format!("{router:?}");
    for route in [
        "GET /",
        "POST /products",
        "GET /products",
        "PATCH /products",
    ] {
        assert!(registered.contains(&format!("\"{route}\"")), "{registered}");
    }
    Ok(())
}
