# Security Guide

This page describes the security configuration options for SlateDuck's PG-Wire
server, the risks of each configuration, and the recommended mitigations.

## Authentication

SlateDuck supports password-based authentication for PG-Wire connections.
Authentication is configured via the `--auth-user` and `--auth-password` flags
(or the `SLATEDUCK_AUTH_USER` / `SLATEDUCK_AUTH_PASSWORD` environment
variables).

When no `--auth-user` is set, the server accepts all connections without
authentication. This is appropriate for local development and single-host
deployments where network access is already restricted.

## TLS Encryption

TLS is configured via `--tls-cert` and `--tls-key` flags pointing to a PEM
certificate and private-key file respectively. Setting `--tls-required`
refuses all non-TLS connections (including plain-text clients).

## Auth Without TLS — Security Risk

> **Warning:** Enabling password authentication without TLS transmits
> credentials in plaintext over the network.

When SlateDuck starts with `--auth-user` set but without `--tls-cert` /
`--tls-key`, it emits a startup warning:

```
WARN slateduck_pgwire::server: Password authentication is enabled without TLS.
Credentials will be sent in plaintext. Use --tls-cert / --tls-key to enable
TLS, or pass --insecure-no-tls-warning-suppress if this is intentional.
```

Any passive network observer between the client and the server can read the
username and password from the PG-Wire `PasswordMessage` packet.

### Mitigations

| Scenario | Recommended action |
|----------|--------------------|
| Internet-facing or multi-tenant | **Always** enable TLS with `--tls-cert` and `--tls-key`. |
| Private LAN / same host | Acceptable without TLS; consider firewall rules. |
| Development / local loop | No TLS needed; omit `--auth-user` or use `--insecure-no-tls-warning-suppress`. |

### Enabling TLS

```bash
slateduck serve \
  --tls-cert /path/to/cert.pem \
  --tls-key  /path/to/key.pem  \
  --tls-required               \
  --auth-user admin             \
  --auth-password "$PASSWORD"
```

Self-signed certificates work for development. For production, use a
certificate signed by a trusted CA (Let's Encrypt, your organisation's PKI,
etc.).

## Clock Skew and Lease Expiry

Snapshot leases use wall-clock time (`SystemTime::now()`) for expiry checks.
In distributed deployments where multiple clients hold leases against the same
catalog:

- Clock skew between nodes can cause a lease holder to see its lease as expired
  before the catalog server's clock agrees.
- The recommended maximum clock skew is **≤ 5 seconds** for the default 1-hour
  lease TTL.
- Use NTP or a similar time-synchronisation service on all nodes.

Lease logic is tested against a `MockClock` (from `slateduck_core::clock`)
that eliminates real-time dependencies in unit tests.
