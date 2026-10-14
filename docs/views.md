# Server-rendered views

Status: foundation crate `forge-views` ([ADR 0022](adr/0022-server-rendered-views.md)).
The generators (`forge new`, `forge generate scaffold`) and the generated
application templates are a later slice; this guide covers the framework
primitives that exist today. `forge-views` lives in the HTTP adapter ring:
domain and application code never depend on it.

```toml
# Cargo.toml of an application
[dependencies]
forge-views = { path = "..." }
askama = { version = "0.16", default-features = false, features = ["derive", "std"] }

[build-dependencies]
# Only the asset embedding helper: no HTTP or template graph in build.rs.
forge-views = { path = "...", default-features = false, features = ["build"] }
```

## Rendering

```rust
use askama::Template;

#[derive(Template)]
#[template(path = "products/show.html")]
struct ProductPage<'a> {
    name: &'a str,
}

async fn show(_: forge_http::Request) -> forge_http::Response {
    forge_views::html(&ProductPage { name: "Desk" })
}
```

- `html(&view)` answers `200` with `Content-Type: text/html; charset=utf-8`;
  `html_with_status(status, &view)` sets another status (for example `422` when
  a form is shown again with errors).
- Templates whose extension is `.html` are HTML-escaped by default. `{{ value }}`
  never emits markup; raw output needs an explicit, greppable `|safe` on a
  value the code has already sanitized.
- A render failure answers `500` with an empty body. The error is logged with
  `tracing`; template error text is never sent to the client.

## Redirects and Turbo

Turbo requires `303 See Other` after a non-GET submission so the follow-up
request is a `GET`:

```rust
let response = forge_views::see_other("/products/1")?;
```

`see_other` accepts only a path that starts with a single `/`. Absolute URLs,
`//host`, `/\host`, whitespace, control, non-ASCII and backslash characters
are refused (`RedirectError`), which closes open redirects through a
user-supplied `next` parameter. Redirecting to an absolute URL requires an
explicit, configured origin:

```rust
forge_views::see_other_to_origin(url, "https://app.example.com")?;
```

The origin (scheme, host, port) must match exactly; URLs with credentials are
refused.

### Turbo Streams

```rust
use forge_views::{DomId, StreamAction, TurboStreamAction, dom_id, turbo_stream};

let target = DomId::new("products")?;
let row = TurboStreamAction::from_view(StreamAction::Append, target, &row_view)?;
let removed = TurboStreamAction::remove(dom_id::<Product>(id, "product")?);
let response = turbo_stream(&[row, removed]);   // text/vnd.turbo-stream.html
```

- `DomId` is validated as `[A-Za-z][A-Za-z0-9_:-]*` (at most 128 bytes), so it
  cannot close an attribute or open a tag; it is safe in `id="..."` and
  `target="..."`.
- `dom_id::<T>(id, prefix)` is Rails' `dom_id` for UUIDv7 ids:
  `{prefix}_{uuid}`. The prefix must start with a letter.
- Every action except `remove` needs a template; `remove` takes none.
  `from_view` renders an Askama view so user data inside the template is
  escaped. The template string passed to `new` is trusted, already-rendered
  HTML.

## Assets

Assets live in `assets/` (CSS, JavaScript, images, fonts). `build.rs` embeds
them with content digests:

```rust
// build.rs
fn main() {
    if let Err(failure) = forge_views::build::embed_assets("assets") {
        eprintln!("asset embedding failed: {failure}");
        std::process::exit(1);
    }
}
```

```rust
// src/adapters/http/assets.rs
include!(concat!(env!("OUT_DIR"), "/forge_assets.rs")); // defines FORGE_ASSETS

pub fn assets() -> Result<forge_views::AssetSet, forge_views::AssetError> {
    forge_views::AssetSet::new(FORGE_ASSETS) // prefix "/assets"
}
```

Register the routes once and use `path` in templates:

```rust
let assets = assets()?;
assets.register(&mut router)?;
assets.path("css/app.css");   // Some("/assets/css/app-0123456789abcdef.css")
```

Build-time rules (errors name the offending path):

- names starting with `.` (files and directories) are ignored;
- symlinks must resolve to a file inside the asset root; escaping links,
  dangling links and links to directories fail the build;
- names use only ASCII letters, digits, `.`, `_` and `-`;
- extensions must be one of `css`, `js`, `mjs`, `map`, `svg`, `png`, `jpg`,
  `jpeg`, `gif`, `webp`, `ico`, `woff2`, `txt`;
- bounded work: at most 4096 files, 32 MiB per file, 16 directory levels.

The digest is SHA-256 of the contents (`sha2`); the URL carries the first 16
hex characters (`app-<16 hex>.css`) and the `ETag` is the full digest. The
output is sorted, so the table is deterministic.

`AssetSet::register` adds one exact `GET` route per manifest entry. There is
no wildcard route, so `..`, percent-encoded separators (`%2e`, `%2f`),
undigested names, wrong digests, directory URLs and paths outside the prefix
match nothing and answer `404`. `HEAD` answers `405`. Responses carry:

| Header | Value |
| --- | --- |
| `Content-Type` | by extension (closed allowlist) |
| `Cache-Control` | `public, max-age=31536000, immutable` |
| `X-Content-Type-Options` | `nosniff` |
| `ETag` | `"<sha256 hex>"`; a matching `If-None-Match` (including `W/` and `*`) answers `304` with no body |
| `Content-Security-Policy` | SVG only: `default-src 'none'; style-src 'unsafe-inline'; sandbox` |

`AssetSet::new` re-validates the table (legal names, allowlisted extension,
well-formed digest, digested name consistent with the digest, no duplicates),
so a hand-edited or corrupted table fails at startup, not at request time.

Development-time serving from disk, `forge assets precompile` and
`forge dev` belong to the generator slice.

## Import map

```rust
let map = forge_views::ImportMap::builder()
    .pin("@hotwired/turbo", "vendor/turbo.js")
    .pin("@hotwired/stimulus", "vendor/stimulus.js")
    .build(&assets)?;

map.script_tag();    // <script type="importmap">{"imports":{...}}</script>
map.csp_source();    // 'sha256-<base64>'
```

Pins resolve to digested URLs; an unknown logical asset, duplicate or invalid
specifier is an error. The JSON escapes `<`, `>`, `&`, U+2028 and U+2029 so it
cannot end the script element. The element has no nonce; its SHA-256 (of the
exact body) is exposed by `sha256_base64()` and `csp_source()`. Put
`script_tag()` in the layout `<head>` without reformatting it.

Forge does not vendor Turbo or Stimulus yet: downloading third-party
JavaScript needs the project owner's approval (ADR 0022 plans pinned, vendored
ES modules).

## Content-Security-Policy

```rust
let policy = forge_views::ContentSecurityPolicy::strict().with_import_map(&map);
let mut response = forge_views::html(&page);
policy.apply(&mut response);
```

The default policy is:

```text
default-src 'self'; script-src 'self' 'sha256-<import map>'; style-src 'self';
img-src 'self' data:; object-src 'none'; base-uri 'self'; form-action 'self';
frame-ancestors 'none'
```

(on one line in the header). There is no `unsafe-inline` and no
`unsafe-eval`: inline event handlers and `style=""` attributes are blocked by
design. `apply` also sets `Referrer-Policy: strict-origin-when-cross-origin`.
Extra inline scripts can be allowed only by hash (`with_script_hash`, which
validates the base64 digest so policy syntax cannot be injected).

## Forms and CSRF

- `csrf_meta(&token)` renders `<meta name="csrf-token" content="...">` for the
  layout `<head>`; Turbo and `fetch` code send it in the `x-csrf-token` header
  (`CSRF_HEADER_NAME`, the header the generated applications' CSRF
  enforcement reads).
- `csrf_field(&token)` renders `<input type="hidden" name="csrf_token" ...>`
  for every non-GET form (`CSRF_FIELD_NAME`).
- Both take a `forge_auth::CsrfToken` and escape the value. They return
  markup: insert them with `|safe`.

`CSRF_FIELD_NAME` is defined here because ADR 0022 names the meta tag and the
header but not the form field. The server-side check for HTML form posts must
read this field with the same rules as the header (exactly one canonical
token, digest compared against the session); the enforcement code in
generated applications currently reads only the header and is extended in the
generator slice.

### Method override

HTML forms can only submit `GET` and `POST`. A form may carry a `_method`
field (`METHOD_FIELD_NAME`) and the handler layer resolves it with:

```rust
let method = forge_views::method_override(csrf_verified, request.method(), field_value);
```

Contract:

1. Honored only when the request method is `POST`; a `GET` (or any other
   method) with `_method` is never changed.
2. Honored only for the values `PATCH`, `PUT` and `DELETE` (ASCII
   case-insensitive). `GET`, `POST`, `TRACE`, unknown and empty values leave
   the request as `POST`.
3. Call it only after CSRF validation. The `CsrfVerified` witness parameter
   makes that visible in code: create it with `CsrfVerified::after_check()`
   in the code path that follows a successful token check. A tunnelled
   `DELETE` that skipped CSRF would let any site delete data on behalf of a
   logged-in user.

`forge-views` does not parse request bodies; the `_method` value comes from
the form decoding the application already performs.

## Not in this slice

Layouts and partial conventions, `forge new` / scaffold generators, flash
messages, the vendored Turbo and Stimulus modules, development-mode asset
serving, and the `forge::views` facade re-export.
