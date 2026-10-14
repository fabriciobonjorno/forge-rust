# Vendored Stimulus

Unmodified ES module build of [Stimulus](https://stimulus.hotwired.dev/)
(Basecamp, MIT, see `LICENSE.md`), served to generated applications through the
import map (ADR 0022). No Node toolchain is required.

| Field | Value |
| --- | --- |
| Package | `@hotwired/stimulus` 3.2.2 |
| Source | https://registry.npmjs.org/@hotwired/stimulus/-/stimulus-3.2.2.tgz |
| npm integrity (tarball) | `sha512-eGeIqNOQpXoPAIP7tC1+1Yc1yl1xnwYqg+3mzqxyrbE5pg5YFBZcA6YoTiByJB6DKAEsiWtl6tjTJS4IYtbB7A==` (verified before vendoring) |
| File | `package/dist/stimulus.js` (88,680 bytes, no imports) |
| SHA-256 | `23deecdac6f36c08e8f39fbed6b27f60c850cacad3a34eb71194462b7b1647f3` |
| Vendored | 2026-10-14, approved by the project owner |

The upstream 3.2.2 tarball ships a file whose banner still reads
"Stimulus 3.2.1"; the bytes above are exactly what npm publishes for 3.2.2.

Updating: download the new tarball from the npm registry, verify its
`dist.integrity`, replace `stimulus.js`/`LICENSE.md` without edits, and update
this table and `STIMULUS_SHA256` in `src/vendor.rs` (its test fails on any
mismatch).
