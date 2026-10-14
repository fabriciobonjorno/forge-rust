//! PostgreSQL integration tests: the repository contract against
//! `PgProductRepository`, and the HTTP handlers over the production wiring.
//!
//! Each test gets its own migrated database from `FORGE_TEST_DATABASE_URL`:
//!
//! ```text
//! FORGE_TEST_DATABASE_URL=postgres://postgres:forge@127.0.0.1:5432/postgres \
//!     cargo test -- --include-ignored
//! ```

#[path = "support/contract.rs"]
mod contract;

use std::{error::Error, future::Future};

use app::{
    adapters::{http::products, persistence::PgProductRepository},
    bootstrap,
};
use forge::http::{Method, Request, StatusCode, Uri, header};
use forge_db::{Migrator, testing::PgTestDatabase};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn Error>>;

/// The application's migrations, exactly as embedded in the binary.
static MIGRATOR: Migrator = sqlx::migrate!();

/// Runs `scenario` against a fresh database, then drops it even when the
/// scenario failed.
async fn with_database<F, Fut>(scenario: F) -> TestResult
where
    F: FnOnce(PgTestDatabase) -> Fut,
    Fut: Future<Output = (PgTestDatabase, TestResult)>,
{
    let test_database = PgTestDatabase::create(&MIGRATOR).await?;
    let (test_database, outcome) = scenario(test_database).await;
    test_database.cleanup().await?;
    outcome
}

/// Runs a contract scenario against `PgProductRepository` on a fresh
/// database.
async fn contract<F, Fut>(scenario: F) -> TestResult
where
    F: FnOnce(PgProductRepository) -> Fut,
    Fut: Future<Output = TestResult>,
{
    with_database(|test_database| async move {
        let repository = PgProductRepository::new(test_database.database());
        let outcome = scenario(repository).await;
        (test_database, outcome)
    })
    .await
}

#[tokio::test]
#[ignore = "requires PostgreSQL (FORGE_TEST_DATABASE_URL)"]
async fn pg_contract_insert_then_find() -> TestResult {
    contract(|repository| async move { contract::insert_then_find(&repository).await }).await
}

#[tokio::test]
#[ignore = "requires PostgreSQL (FORGE_TEST_DATABASE_URL)"]
async fn pg_contract_duplicate_sku_is_rejected() -> TestResult {
    contract(|repository| async move { contract::duplicate_sku_is_rejected(&repository).await })
        .await
}

#[tokio::test]
#[ignore = "requires PostgreSQL (FORGE_TEST_DATABASE_URL)"]
async fn pg_contract_update_advances_the_version() -> TestResult {
    contract(|repository| async move { contract::update_advances_the_version(&repository).await })
        .await
}

#[tokio::test]
#[ignore = "requires PostgreSQL (FORGE_TEST_DATABASE_URL)"]
async fn pg_contract_stale_update_conflicts() -> TestResult {
    contract(|repository| async move { contract::stale_update_conflicts(&repository).await }).await
}

#[tokio::test]
#[ignore = "requires PostgreSQL (FORGE_TEST_DATABASE_URL)"]
async fn pg_contract_update_of_missing_product_is_not_found() -> TestResult {
    contract(|repository| async move {
        contract::update_of_missing_product_is_not_found(&repository).await
    })
    .await
}

#[tokio::test]
#[ignore = "requires PostgreSQL (FORGE_TEST_DATABASE_URL)"]
async fn pg_contract_cursor_pagination_visits_every_product_once() -> TestResult {
    contract(|repository| async move {
        contract::cursor_pagination_visits_every_product_once(&repository).await
    })
    .await
}

#[tokio::test]
#[ignore = "requires PostgreSQL (FORGE_TEST_DATABASE_URL)"]
async fn check_constraints_back_up_the_domain_rules() -> TestResult {
    with_database(|test_database| async move {
        let outcome = async {
            for (sku, name, cents, currency) in [
                ("lower", "Mug", 1_i64, "EUR"),
                ("MUG-1", "", 1, "EUR"),
                ("MUG-2", "Mug", -1, "EUR"),
                ("MUG-3", "Mug", 1, "eur"),
            ] {
                let result = sqlx::query(
                    "INSERT INTO products (id, sku, name, price_cents, currency) \
                     VALUES ($1, $2, $3, $4, $5)",
                )
                .bind(forge::core::Id::<()>::new().into_uuid())
                .bind(sku)
                .bind(name)
                .bind(cents)
                .bind(currency)
                .execute(test_database.database().pool())
                .await;
                let error = result.err().map(forge_db::DbError::from);
                assert!(
                    matches!(error, Some(forge_db::DbError::CheckViolation { .. })),
                    "{sku}: {error:?}"
                );
            }
            Ok(())
        }
        .await;
        (test_database, outcome)
    })
    .await
}

/// Drives the handlers exactly as `bootstrap::compose` wires them. The Forge
/// router cannot dispatch in-process, so handlers are called directly.
#[tokio::test]
#[ignore = "requires PostgreSQL (FORGE_TEST_DATABASE_URL)"]
async fn http_flow_over_postgres() -> TestResult {
    with_database(|test_database| async move {
        let outcome = http_flow(bootstrap::product_catalog(test_database.database())).await;
        (test_database, outcome)
    })
    .await
}

async fn http_flow(
    catalog: app::application::use_cases::ProductCatalog<PgProductRepository>,
) -> TestResult {
    let request = |method: Method, uri: &str, body: String| -> Result<Request, Box<dyn Error>> {
        Ok(Request::new_for_test(
            method,
            uri.parse::<Uri>()?,
            Default::default(),
            body.into(),
            "127.0.0.1:0".parse()?,
            uuid::Uuid::now_v7(),
        ))
    };
    let body = |response: &forge::http::Response| serde_json::from_slice::<Value>(response.body());
    let mug = json!({"sku": "mug-1", "name": "Mug", "price": {"cents": 1299, "currency": "EUR"}});

    let created = products::create_product(
        &catalog,
        request(Method::POST, "/products", mug.to_string())?,
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let location = created
        .headers()
        .get(header::LOCATION)
        .ok_or("Location header")?
        .to_str()?
        .to_owned();
    let product = body(&created)?;
    assert_eq!(product["sku"], "MUG-1");

    let duplicate = products::create_product(
        &catalog,
        request(Method::POST, "/products", mug.to_string())?,
    )
    .await;
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    assert_eq!(body(&duplicate)?["code"], "sku_taken");

    let fetched =
        products::get_products(&catalog, request(Method::GET, &location, String::new())?).await;
    assert_eq!(fetched.status(), StatusCode::OK);
    assert_eq!(body(&fetched)?, product);

    let rename = json!({"expected_version": 1, "name": "Cup"}).to_string();
    let renamed =
        products::patch_product(&catalog, request(Method::PATCH, &location, rename.clone())?).await;
    assert_eq!(renamed.status(), StatusCode::OK);
    assert_eq!(body(&renamed)?["version"], 2);

    let stale = products::patch_product(&catalog, request(Method::PATCH, &location, rename)?).await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(body(&stale)?["code"], "concurrent_modification");

    let listed =
        products::get_products(&catalog, request(Method::GET, "/products", String::new())?).await;
    let listed = body(&listed)?;
    assert_eq!(listed["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(listed["items"][0]["name"], "Cup");
    assert_eq!(listed["next_cursor"], Value::Null);
    Ok(())
}
