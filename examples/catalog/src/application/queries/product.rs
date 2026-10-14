//! Queries of the product catalog.

use forge::core::{Id, PageRequest};

use crate::domain::entities::Product;

/// Loads one product.
#[derive(Debug)]
pub struct GetProduct {
    /// Product to load.
    pub id: Id<Product>,
}

/// Lists products newest first, one cursor page at a time.
#[derive(Debug, Default)]
pub struct ListProducts {
    /// Page size and cursor.
    pub page: PageRequest<Product>,
}
