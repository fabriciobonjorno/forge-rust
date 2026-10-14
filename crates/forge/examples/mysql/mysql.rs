//! Forge application backed by MySQL or MariaDB.
//!
//! ```text
//! docker run -d --rm --name forge-example-mysql -e MYSQL_ROOT_PASSWORD=forge \
//!     -e MYSQL_DATABASE=example -p 127.0.0.1:3306:3306 mysql:8.4
//! export FORGE_DATABASE_URL=mysql://root:forge@127.0.0.1:3306/example
//! cargo run -p forge --no-default-features --features mysql --example mysql -- migrate
//! cargo run -p forge --no-default-features --features mysql --example mysql
//!
//! curl -X POST http://127.0.0.1:3000/notes -d '{"title":"hello"}'
//! curl http://127.0.0.1:3000/notes
//! curl -i http://127.0.0.1:3000/health/ready
//! ```
//!
//! `Id<T>` columns are `BINARY(16)`; `mariadb://` URLs work as well.

use std::process::ExitCode;

use forge::{
    App,
    core::{ForgeError, Id, Page, PageLimit},
    db::{DbError, MySqlDatabase},
    http::{Method, Request, Response, StatusCode},
};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, migrate};
use sqlx_crate as sqlx;
use forge_core::IntoForgeError;

enum Note {}

#[derive(Deserialize)]
struct CreateNote {
    title: String,
}

#[derive(Serialize, sqlx::FromRow)]
struct NoteView {
    id: Id<Note>,
    title: String,
}

fn main() -> ExitCode {
    App::new("mysql-example")
        .version(env!("CARGO_PKG_VERSION"))
        .mysql(::sqlx::migrate!("examples/migrations/mysql"))
        .compose(|composition| {
            let database = composition.mysql()?.clone();
            let router = composition.router();
            let db = database.clone();
            router.route(Method::POST, "/notes", move |request: Request| {
                let db = db.clone();
                async move { create_note(&db, &request).await }
            })?;
            router.route(Method::GET, "/notes", move |_| {
                let db = database.clone();
                async move { list_notes(&db).await }
            })?;
            Ok(())
        })
        .run()
}

async fn create_note(database: &MySqlDatabase, request: &Request) -> Response {
    let Ok(input) = request.json::<CreateNote>() else {
        return Response::new(StatusCode::BAD_REQUEST, "expected {\"title\": \"...\"}\n");
    };
    let id = Id::<Note>::new();
    let inserted = sqlx::query("INSERT INTO notes (id, title) VALUES (?, ?)")
        .bind(id)
        .bind(&input.title)
        .execute(database.pool())
        .await;
    match inserted {
        Ok(_) => json(
            StatusCode::CREATED,
            &NoteView {
                id,
                title: input.title,
            },
        ),
        Err(error) => error_response(DbError::from(error)),
    }
}

/// First page of notes, newest first (UUIDv7 ids are time-ordered).
async fn list_notes(database: &MySqlDatabase) -> Response {
    let limit = PageLimit::DEFAULT;
    let rows =
        sqlx::query_as::<_, NoteView>("SELECT id, title FROM notes ORDER BY id DESC LIMIT ?")
            .bind(limit.fetch_size())
            .fetch_all(database.pool())
            .await;
    match rows {
        Ok(rows) => json(
            StatusCode::OK,
            &Page::from_overfetched(rows, limit, |note: &NoteView| note.id),
        ),
        Err(error) => error_response(DbError::from(error)),
    }
}

/// Maps a classified database error to its caller-safe HTTP representation.
fn error_response(error: DbError) -> Response {
    let error = error.into_forge_error();
    let status =
        StatusCode::from_u16(error.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    json(status, &error.report())
}

fn json(status: StatusCode, value: &impl Serialize) -> Response {
    Response::json(status, value)
        .unwrap_or_else(|_| Response::empty(StatusCode::INTERNAL_SERVER_ERROR))
}
