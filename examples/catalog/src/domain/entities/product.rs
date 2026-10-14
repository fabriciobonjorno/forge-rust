//! The product aggregate.

use chrono::{DateTime, Utc};
use forge::core::{Id, Version};

use crate::domain::{
    errors::ProductError,
    value_objects::{Money, ProductName, Sku},
};

/// A product that has been validated but not stored yet.
///
/// Its identifier is assigned here, before persistence, so the caller knows
/// it without a database round trip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewProduct {
    id: Id<Product>,
    sku: Sku,
    name: ProductName,
    price: Money,
}

impl NewProduct {
    /// Creates a product with a fresh UUIDv7 identifier.
    #[must_use]
    pub fn new(sku: Sku, name: ProductName, price: Money) -> Self {
        Self {
            id: Id::new(),
            sku,
            name,
            price,
        }
    }

    /// Identifier the product will be stored under.
    #[must_use]
    pub const fn id(&self) -> Id<Product> {
        self.id
    }

    /// Stock keeping unit.
    #[must_use]
    pub const fn sku(&self) -> &Sku {
        &self.sku
    }

    /// Display name.
    #[must_use]
    pub const fn name(&self) -> &ProductName {
        &self.name
    }

    /// Price.
    #[must_use]
    pub const fn price(&self) -> &Money {
        &self.price
    }
}

/// A stored product.
///
/// The SKU and the price currency never change. `version` supports
/// optimistic locking and is advanced by the repository on every update.
/// `created_at` and `updated_at` are persistence metadata recorded by the
/// repository; they are informational and not trusted business time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Product {
    id: Id<Product>,
    sku: Sku,
    name: ProductName,
    price: Money,
    version: Version,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl Product {
    /// Rebuilds a product from stored state.
    ///
    /// Intended for repositories: every value is already validated by its
    /// type, and no business transition is applied.
    #[must_use]
    pub const fn restore(
        id: Id<Self>,
        sku: Sku,
        name: ProductName,
        price: Money,
        version: Version,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            sku,
            name,
            price,
            version,
            created_at,
            updated_at,
        }
    }

    /// Identifier.
    #[must_use]
    pub const fn id(&self) -> Id<Self> {
        self.id
    }

    /// Stock keeping unit.
    #[must_use]
    pub const fn sku(&self) -> &Sku {
        &self.sku
    }

    /// Display name.
    #[must_use]
    pub const fn name(&self) -> &ProductName {
        &self.name
    }

    /// Current price.
    #[must_use]
    pub const fn price(&self) -> &Money {
        &self.price
    }

    /// Version of the stored state this value was read from.
    #[must_use]
    pub const fn version(&self) -> Version {
        self.version
    }

    /// When the product was first stored.
    #[must_use]
    pub const fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    /// When the product was last stored.
    #[must_use]
    pub const fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }

    /// Gives the product a new name.
    pub fn rename(&mut self, name: ProductName) {
        self.name = name;
    }

    /// Changes the price; the currency must stay the same.
    pub fn change_price(&mut self, price: Money) -> Result<(), ProductError> {
        if price.currency() != self.price.currency() {
            return Err(ProductError::CurrencyMismatch);
        }
        self.price = price;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value_objects::Currency;

    fn product() -> Result<Product, ProductError> {
        let draft = NewProduct::new(
            Sku::parse("mug-1")?,
            ProductName::parse("Mug")?,
            Money::new(1_299, Currency::parse("EUR")?)?,
        );
        let now = Utc::now();
        Ok(Product::restore(
            draft.id(),
            draft.sku().clone(),
            draft.name().clone(),
            draft.price().clone(),
            Version::INITIAL,
            now,
            now,
        ))
    }

    #[test]
    fn new_products_get_distinct_identifiers() -> Result<(), ProductError> {
        let make = || -> Result<NewProduct, ProductError> {
            Ok(NewProduct::new(
                Sku::parse("ABC")?,
                ProductName::parse("A")?,
                Money::new(0, Currency::parse("EUR")?)?,
            ))
        };
        assert_ne!(make()?.id(), make()?.id());
        Ok(())
    }

    #[test]
    fn rename_replaces_the_name_only() -> Result<(), ProductError> {
        let mut product = product()?;
        let before = product.clone();

        product.rename(ProductName::parse("Big mug")?);

        assert_eq!(product.name().as_str(), "Big mug");
        assert_eq!(product.sku(), before.sku());
        assert_eq!(product.price(), before.price());
        assert_eq!(product.version(), before.version());
        Ok(())
    }

    #[test]
    fn change_price_keeps_the_currency() -> Result<(), ProductError> {
        let mut product = product()?;

        product.change_price(Money::new(1_499, Currency::parse("eur")?)?)?;
        assert_eq!(product.price().cents(), 1_499);

        let rejected = product.change_price(Money::new(1, Currency::parse("USD")?)?);
        assert_eq!(rejected, Err(ProductError::CurrencyMismatch));
        assert_eq!(product.price().cents(), 1_499);
        Ok(())
    }
}
