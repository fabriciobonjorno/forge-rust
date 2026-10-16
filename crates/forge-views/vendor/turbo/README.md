# Vendored Turbo

Unmodified ES module build of [Turbo](https://turbo.hotwired.dev/)
(37signals, MIT, see `LICENSE`), served to generated applications through the
import map (ADR 0022). No Node toolchain is required.

| Field | Value |
| --- | --- |
| Package | `@hotwired/turbo` 8.0.23 |
| Source | https://registry.npmjs.org/@hotwired/turbo/-/turbo-8.0.23.tgz |
| npm integrity (tarball) | `sha512-GZ7cijxEZ6Ig71u7rD6LHaRv/wcE/hNsc+nEfiWOkLNqUgLOwo5MNGWOy5ZV9ZUDSiQx1no7YxjTNnT4O6//cQ==` (verified before vendoring) |
| File | `package/dist/turbo.es2017-esm.js` (203,701 bytes, no imports) |
| SHA-256 | `b9d35d123a07614f55eaaf993f74d687a503ae41ba50ef835aafa18dbb265a13` |
| License text | `MIT-LICENSE` at tag `v8.0.23` of https://github.com/hotwired/turbo (the npm tarball does not include it) |
| Vendored | 2026-10-16, approved by the project owner |

Updating: download the new tarball from the npm registry, verify its
`dist.integrity`, replace `turbo.js` without edits, refresh `LICENSE` from the
matching upstream tag, and update this table and `TURBO_SHA256` in
`src/vendor.rs` (its test fails on any mismatch).
