//! MySQL 8.4+ and MariaDB 11.4+ backend (one wire protocol, one backend).

use forge_config::DatabaseScheme;
use sqlx::MySql;

use crate::{Backend, backend::sealed::Hooks};

impl Hooks for MySql {
    const SCHEME: DatabaseScheme = DatabaseScheme::MySql;
    const TEST_URL_ENV: &'static str = "FORGE_TEST_MYSQL_URL";

    #[cfg(feature = "testing")]
    fn provision(
        admin_env: &str,
    ) -> impl std::future::Future<Output = Result<crate::testing::Provision, crate::DbError>> + Send
    {
        crate::testing::provision_server(admin_env, create_database, drop_database)
    }
}

impl Backend for MySql {
    const SCHEME: DatabaseScheme = DatabaseScheme::MySql;
}

#[cfg(feature = "testing")]
fn create_database(admin: forge_config::SecretString, name: String) -> crate::testing::BoxFuture {
    Box::pin(async move {
        crate::testing::execute_admin::<MySql>(
            &admin,
            format!("CREATE DATABASE `{name}` CHARACTER SET utf8mb4"),
        )
        .await
    })
}

#[cfg(feature = "testing")]
fn drop_database(admin: forge_config::SecretString, name: String) -> crate::testing::BoxFuture {
    Box::pin(async move {
        crate::testing::execute_admin::<MySql>(&admin, format!("DROP DATABASE IF EXISTS `{name}`"))
            .await
    })
}
