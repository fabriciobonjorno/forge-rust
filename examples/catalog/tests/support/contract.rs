//! Contract suite every [`ProductRepository`] implementation must pass.
//!
//! Each scenario expects an empty repository. `tests/catalog.rs` runs the
//! suite against the in-memory double and `tests/postgres.rs` against
//! `PgProductRepository`, so the double cannot drift from the real adapter.

use std::error::Error;

use app::{
    application::ports::{ProductRepository, RepositoryError},
    domain::{
        entities::{NewProduct, Product},
        errors::ProductError,
        value_objects::{Currency, Money, ProductName, Sku},
    },
};
use forge::core::{Id, PageLimit, PageRequest, Version};

/// Result of a contract scenario.
pub type ContractResult = Result<(), Box<dyn Error>>;

/// A valid product with the given SKU, priced in EUR.
pub fn new_product(sku: &str, cents: i64) -> Result<NewProduct, ProductError> {
    Ok(NewProduct::new(
        Sku::parse(sku)?,
        ProductName::parse(&format!("Product {sku}"))?,
        Money::new(cents, Currency::parse("EUR")?)?,
    ))
}

/// `insert` stores version 1 with equal timestamps; `find` returns it.
pub async fn insert_then_find<R: ProductRepository>(repository: &R) -> ContractResult {
    let draft = new_product("MUG-1", 1_299)?;

    let stored = repository.insert(&draft).await?;

    assert_eq!(stored.id(), draft.id());
    assert_eq!(stored.sku(), draft.sku());
    assert_eq!(stored.name(), draft.name());
    assert_eq!(stored.price(), draft.price());
    assert_eq!(stored.version(), Version::INITIAL);
    assert_eq!(stored.created_at(), stored.updated_at());
    assert_eq!(repository.find(draft.id()).await?, Some(stored));
    assert_eq!(repository.find(Id::new()).await?, None);
    Ok(())
}

/// A second product with the same SKU is rejected with `SkuTaken`.
pub async fn duplicate_sku_is_rejected<R: ProductRepository>(repository: &R) -> ContractResult {
    repository.insert(&new_product("MUG-1", 100)?).await?;

    let duplicate = new_product("mug-1", 200)?;
    let result = repository.insert(&duplicate).await;

    assert!(
        matches!(result, Err(RepositoryError::SkuTaken)),
        "{result:?}"
    );
    assert_eq!(repository.find(duplicate.id()).await?, None);
    Ok(())
}

/// `update` writes name and price, advances the version and keeps the rest.
pub async fn update_advances_the_version<R: ProductRepository>(repository: &R) -> ContractResult {
    let mut product = repository.insert(&new_product("MUG-1", 100)?).await?;
    product.rename(ProductName::parse("Renamed")?);
    product.change_price(Money::new(250, Currency::parse("EUR")?)?)?;

    let updated = repository.update(&product).await?;

    assert_eq!(updated.id(), product.id());
    assert_eq!(updated.sku(), product.sku());
    assert_eq!(updated.name().as_str(), "Renamed");
    assert_eq!(updated.price().cents(), 250);
    assert_eq!(updated.version(), product.version().next()?);
    assert_eq!(updated.created_at(), product.created_at());
    assert!(updated.updated_at() >= product.updated_at());
    assert_eq!(repository.find(product.id()).await?, Some(updated));
    Ok(())
}

/// Updating from a stale copy fails with `VersionConflict` and changes
/// nothing.
pub async fn stale_update_conflicts<R: ProductRepository>(repository: &R) -> ContractResult {
    let original = repository.insert(&new_product("MUG-1", 100)?).await?;
    let mut first = original.clone();
    first.rename(ProductName::parse("First writer")?);
    let current = repository.update(&first).await?;

    let mut stale = original;
    stale.rename(ProductName::parse("Second writer")?);
    let result = repository.update(&stale).await;

    assert!(
        matches!(result, Err(RepositoryError::VersionConflict)),
        "{result:?}"
    );
    assert_eq!(repository.find(current.id()).await?, Some(current));
    Ok(())
}

/// Updating a product that was never stored fails with `NotFound`.
pub async fn update_of_missing_product_is_not_found<R: ProductRepository>(
    repository: &R,
) -> ContractResult {
    let draft = new_product("MUG-1", 100)?;
    let never_stored = Product::restore(
        draft.id(),
        draft.sku().clone(),
        draft.name().clone(),
        draft.price().clone(),
        Version::INITIAL,
        chrono::Utc::now(),
        chrono::Utc::now(),
    );

    let result = repository.update(&never_stored).await;

    assert!(
        matches!(result, Err(RepositoryError::NotFound)),
        "{result:?}"
    );
    Ok(())
}

/// Following `next_cursor` visits every product exactly once, newest first,
/// across three pages (3 + 3 + 1), with no trailing empty page.
pub async fn cursor_pagination_visits_every_product_once<R: ProductRepository>(
    repository: &R,
) -> ContractResult {
    let mut expected = Vec::new();
    for index in 0..7 {
        let product = repository
            .insert(&new_product(&format!("SKU-{index}"), index)?)
            .await?;
        expected.push(product.id());
    }
    expected.sort_by(|left, right| right.cmp(left));

    let limit = PageLimit::new(3)?;
    let mut request = PageRequest::<Product> { limit, after: None };
    let mut page_sizes = Vec::new();
    let mut seen = Vec::new();
    loop {
        let page = repository.list(request).await?;
        page_sizes.push(page.items.len());
        seen.extend(page.items.iter().map(Product::id));
        match page.next_cursor {
            Some(cursor) => {
                assert_eq!(Some(cursor), page.items.last().map(Product::id));
                request.after = Some(cursor);
            }
            None => break,
        }
    }

    assert_eq!(page_sizes, [3, 3, 1]);
    assert_eq!(seen, expected);
    Ok(())
}
