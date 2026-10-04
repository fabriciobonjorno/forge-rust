# ADR 0016: Explicit trusted-proxy client address resolution

- Status: Accepted
- Date: 2026-09-28
- Refines: [ADR 0015](0015-login-and-origin-credential-throttling.md)

## Context

Origin-scoped security controls need the address of the connecting client when
Forge is deployed behind reverse proxies. Forwarding headers are ordinary
untrusted request input unless the immediate TCP peer is explicitly trusted.
Trusting `X-Forwarded-For` unconditionally would let a direct client spoof its
origin and bypass or exhaust origin-scoped budgets.

## Decision

- `FORGE_TRUSTED_PROXIES` accepts a comma-separated list of IPv4/IPv6 CIDR
  networks. It is empty by default; use `/32` or `/128` to trust one address.
- Only the `X-Forwarded-For` header is interpreted. Other forwarding headers,
  including `Forwarded` and `X-Real-IP`, are ignored.
- If the immediate TCP peer is outside configured networks, Forge ignores
  `X-Forwarded-For` and uses the peer IP as the client IP.
- If the peer is trusted and the header is present, Forge walks the list from
  right to left, accepting a new hop only while the previously observed hop is
  trusted. It stops at the first untrusted address; values further left cannot
  override that boundary.
- If a trusted peer omits the header, Forge falls back to its observed peer IP.
  A malformed header or a chain exceeding 4096 bytes / 32 entries is rejected
  with HTTP 400 before the application handler runs.
- `Request::remote_addr()` remains the actual TCP socket peer. The distinct
  `Request::client_addr()` value is available to application adapters; the
  generated login throttle accepts that resolved IP instead of a socket address.
- Operators must configure only proxy egress networks and ensure each trusted
  proxy appends the address of its observed upstream peer. Network presence
  alone does not establish identity or authorize a tenant.

## Consequences

- Default deployments keep the existing peer-address behavior and ignore
  user-supplied forwarding headers.
- Applications behind trusted proxies can apply origin controls to the
  original client without trusting arbitrary header input.
- Misconfigured CIDRs can enable spoofing; configuration is explicit and
  validated at startup. The application never infers proxy ranges from bind
  addresses, private-network status, or deployment environment.
- `ipnet` is used only in `forge-config` to parse and match CIDR values; its
  third-party network types do not appear in the public contract.
- The parser intentionally has bounded size and hop count. Applications that
  require another proxy header format need a separately reviewed policy.
