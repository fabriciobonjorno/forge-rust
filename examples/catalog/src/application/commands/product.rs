//! Commands of the product catalog. Fields are domain value objects, so a
//! command that exists is already valid input.

use forge::core::{Id, Version};

use crate::domain::{
    entities::Product,
    value_objects::{Money, ProductName, Sku},
};

/// Adds a product to the catalog.
#[derive(Debug)]
pub struct CreateProduct {
    /// Unique stock keeping unit.
    pub sku: Sku,
    /// Display name.
    pub name: ProductName,
    /// Initial price; its currency is fixed for the product's lifetime.
    pub price: Money,
}

/// Renames the product last read at `expected_version`.
#[derive(Debug)]
pub struct RenameProduct {
    /// Product to rename.
    pub id: Id<Product>,
    /// Version the client based the change on.
    pub expected_version: Version,
    /// New name.
    pub name: ProductName,
}

/// Changes the price of the product last read at `expected_version`.
#[derive(Debug)]
pub struct ChangePrice {
    /// Product to reprice.
    pub id: Id<Product>,
    /// Version the client based the change on.
    pub expected_version: Version,
    /// New price, in the product's currency.
    pub price: Money,
}
