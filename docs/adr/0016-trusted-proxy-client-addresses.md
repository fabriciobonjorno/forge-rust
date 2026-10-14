# ADR 0016: Explicit trusted-proxy client address resolution

- Status: Accepted
- Date: 2026-10-04
- Refines: [ADR 0015](0015-persistent-login-throttling.md)

## Context

Login throttling by TCP peer address is safe for directly connected clients but
aggregates every user behind one reverse proxy. Reading `X-Forwarded-For`
without authenticating the proxy boundary lets direct clients choose their own
rate-limit identity. Both defaults are unsuitable for a generated application
that wants reverse-proxy deployments to work securely.

## Decision

Forge provides the following explicit boundary:

- `FORGE_TRUSTED_PROXIES` accepts at most 64 comma-separated IPv4/IPv6
  addresses or CIDRs. It defaults to empty; an empty or missing list trusts no
  forwarding headers.
- `Request::client_ip` returns the socket peer unchanged when that peer is not
  in the configured CIDRs.
- For a configured peer, it strictly parses every `X-Forwarded-For` value,
  combines multiple header fields in order, limits the chain to 4096 bytes and
  32 hops, then walks from the rightmost address toward the left. It stops at
  the first address outside the trusted CIDRs.
- Missing, malformed, oversized or overlong forwarding data from a configured
  proxy returns a typed error. It never silently falls back to the proxy address.
- Only IP literals are accepted; hostnames, ports, obfuscated identifiers and
  RFC `Forwarded` values are not parsed as `X-Forwarded-For` addresses.

Application adapters must explicitly pass the validated
`config.server.trusted_proxies` to `Request::client_ip`. They must not trust
forwarding headers directly. Proxy operators must ensure their proxy appends the
actual peer address in standard `X-Forwarded-For` order. A proxy that is not in
the list is treated as an ordinary peer.

## Consequences

- Direct deployments remain secure without additional configuration.
- Multi-hop proxy deployments can derive a rate-limit identity without trusting
  arbitrary client-supplied headers.
- Operators own the trust boundary and must use narrow, controlled proxy CIDRs.
- This helper does not automatically install middleware or make generated
  authentication routes proxy-ready; handlers must call it explicitly.
- Keyed digests for persisted throttle keys remain a separate hardening item.
