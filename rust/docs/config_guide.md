# Rust Mumble Client – Configuration Guide

This guide explains every configuration option exposed by `ClientConfig` and
the TLS settings available in the Rust Mumble client library.

---

## `ClientConfig`

`ClientConfig` is constructed with a builder pattern.  Only the **required**
fields (`host` and `username`) must be supplied; everything else has a sensible
default.

```rust
use mumble_client::ClientConfig;

let config = ClientConfig::new("mumble.example.com", "MyBot")
    .with_port(64738)
    .with_password("serverpassword")
    .with_token("vip-access-token")
    .accept_invalid_certs(); // ⚠ testing only
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `host` | `String` | *(required)* | Hostname or IP address of the Mumble server. |
| `port` | `u16` | `64738` | TCP port. The Mumble default is `64738`. |
| `username` | `String` | *(required)* | Username displayed to other users on the server. |
| `password` | `Option<String>` | `None` | Server password or registered-account password. Only needed when the server or account requires one. |
| `tokens` | `Vec<String>` | `[]` | ACL access tokens. These act as group passwords granting access to ACL groups without requiring full registration. Multiple tokens can be supplied. |
| `accept_invalid_certs` | `bool` | `false` | Disable TLS certificate verification. **Never use in production.** Useful when connecting to a local test server with a self-signed certificate. |

---

## Connection parameters

### Host

Supply a DNS hostname or a raw IPv4/IPv6 address.  The value is used both as
the TCP endpoint and as the TLS SNI server name (when `accept_invalid_certs`
is `false`).

```rust
// DNS name
ClientConfig::new("mumble.example.com", "bot");

// IPv4 address (cert verification must be disabled when using an IP)
ClientConfig::new("192.168.1.10", "bot").accept_invalid_certs();
```

### Port

The Mumble default port is **64738**.  Override it with `.with_port(port)`:

```rust
ClientConfig::new("mumble.example.com", "bot").with_port(64739);
```

---

## Authentication

### Username

The username is an arbitrary UTF-8 string.  The server may impose additional
restrictions (e.g. uniqueness, maximum length).

### Password

Supply a password when:
- the server is password-protected, **or**
- you want to authenticate to a registered account that has a password set.

```rust
ClientConfig::new("mumble.example.com", "RegisteredUser")
    .with_password("mypassword");
```

### Access tokens

Tokens grant access to ACL groups without requiring full server registration.
They are sent to the server as part of the `Authenticate` message.

```rust
ClientConfig::new("mumble.example.com", "vip-bot")
    .with_token("vip-channel-password")
    .with_token("admin-channel-password");
```

---

## TLS configuration

### Default behaviour (production)

By default the library uses `webpki-roots` (Mozilla's root CA bundle) to verify
the server certificate.  This is the correct setting for connecting to any
publicly-hosted Mumble server.

```rust
// No special configuration needed – TLS is always on and verified by default.
let config = ClientConfig::new("mumble.example.com", "bot");
```

### Self-signed / untrusted certificates

When connecting to a local test server or a server with a self-signed
certificate, call `.accept_invalid_certs()`:

```rust
let config = ClientConfig::new("127.0.0.1", "test-bot")
    .with_port(64738)
    .accept_invalid_certs(); // ⚠ disables all TLS verification
```

> **Security warning:** `.accept_invalid_certs()` disables all certificate
> chain and hostname verification.  An attacker who can intercept the TCP
> connection will be able to impersonate the server.  Only use this option
> in controlled test environments.

---

## Runtime configuration

The following settings are hard-coded in the current release but will be
exposed as configuration options in a future version.

| Setting | Current value | Notes |
|---------|---------------|-------|
| Ping interval | 15 seconds | Mumble servers disconnect after 30 s without a ping. |
| Client type | `BOT` (1) | Set to `REGULAR` (0) for end-user clients once audio support is added. |
| Opus | enabled | The library always advertises Opus support in `Authenticate`. |
| CELT versions | none | CELT is not supported. Opus is the recommended codec. |

---

## Environment variables

No environment variables are read by the library at this time.

---

## Logging

The library uses the [`tracing`](https://docs.rs/tracing) crate for structured
logging.  To see log output, initialise a subscriber in your application:

```rust
tracing_subscriber::fmt::init();
```

Log levels used:

| Level | Events |
|-------|--------|
| `INFO` | Connected, ServerSync received (session id, welcome text) |
| `DEBUG` | Individual channel / user state messages, unhandled message types |
| `WARN` | `PermissionDenied` messages |
| `ERROR` | Not used directly; errors are returned via `Result` |
