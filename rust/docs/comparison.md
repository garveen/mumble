# Rust vs C++ Mumble Client – Comparison

This document compares the Rust client library (`rust/`) with the canonical
C++ Mumble client (`src/mumble/`) along several dimensions.

---

## Feature matrix

| Feature | C++ client | Rust library |
|---------|-----------|--------------|
| TCP/TLS connection | ✅ | ✅ |
| Protocol Buffer message framing | ✅ | ✅ |
| Version exchange | ✅ | ✅ |
| Authentication (user / password / tokens) | ✅ | ✅ |
| CryptSetup reception | ✅ | ✅ (stored, UDP not yet implemented) |
| Channel state sync | ✅ | ✅ |
| User state sync | ✅ | ✅ |
| ServerSync | ✅ | ✅ |
| Ping / keepalive | ✅ | ✅ |
| Text messages (send) | ✅ | ✅ |
| Move to channel | ✅ | ✅ |
| WebSocket transport (ws:// / wss://) | ✅ (PR #1) | ❌ planned |
| UDP voice channel | ✅ | ❌ planned |
| OCB-AES128 UDP encryption | ✅ | ❌ planned |
| Opus audio encode/decode | ✅ | ❌ planned |
| CELT audio (legacy) | ✅ | ❌ not planned |
| ACL / permission queries | ✅ | ❌ planned |
| User/channel remove messages | ✅ | ❌ planned |
| UserStats / RequestBlob | ✅ | ❌ planned |
| Server configuration reception | ✅ | ❌ planned |
| Context actions | ✅ | ❌ planned |
| Voice targets (whisper/shout) | ✅ | ❌ planned |
| Client certificate authentication | ✅ | ❌ planned |
| Plugin system | ✅ | ❌ out of scope |
| GUI | ✅ | ❌ out of scope |
| Overlay | ✅ | ❌ out of scope |

---

## Architecture

### C++ client

The C++ client is a full Qt application:

- **`ServerHandler`** owns a `QSslSocket` (TCP/TLS) and optionally a
  `WebSocketConnection`.  It drives the network I/O from Qt's event loop.
- **`Connection`** wraps `QSslSocket` and provides `sendMessage` / slot-based
  reception.
- Protobuf messages are generated from `Mumble.proto` by `protoc`.
- Audio capture and playback are handled by platform-specific `AudioInput` /
  `AudioOutput` classes.
- The entire application is structured around Qt signals/slots.

### Rust library

The Rust library is a minimal async library:

- **`MumbleClient<S>`** is generic over any `AsyncRead + AsyncWrite` stream,
  making it easy to test with in-memory pipes or to swap the transport.
- **`proto`** module handles framing independently of the transport.
- Async I/O is provided by `tokio`; TLS by `tokio-rustls` / `rustls`.
- Protobuf messages are generated at build time by `prost-build`.
- No event loop or GUI framework is assumed.

---

## Protocol compatibility

Both implementations speak the same Mumble protocol (defined in
`src/Mumble.proto`).  The Rust library shares the same `.proto` file.

Version field encoding:
- Legacy v1: `(major << 16) | (minor << 8) | patch` – same in both.
- New v2 (64-bit): `(major << 48) | (minor << 32) | (patch << 16) | build` –
  same in both.

---

## Dependencies

### C++ client (key)

| Library | Purpose |
|---------|---------|
| Qt 6 (Core, Network, Ssl, WebSockets) | Event loop, networking, UI |
| Protobuf / protoc | Protocol Buffers |
| OpenSSL | TLS |
| Opus | Audio codec |
| Boost / POCO (optional) | Various utilities |

### Rust library (key)

| Crate | Purpose |
|-------|---------|
| `tokio` | Async runtime |
| `tokio-rustls` | Async TLS |
| `rustls` | TLS implementation (no OpenSSL dependency) |
| `webpki-roots` | Mozilla root CA bundle |
| `prost` | Protocol Buffers runtime |
| `prost-build` | Compile-time `.proto` code generation |
| `bytes` | Zero-copy byte buffers |
| `thiserror` | Ergonomic error types |
| `tracing` | Structured logging |

---

## Safety & correctness

| Property | C++ client | Rust library |
|----------|-----------|--------------|
| Memory safety | Programmer responsibility | Guaranteed by the borrow checker |
| Thread safety | Qt threading model | `Send`/`Sync` traits enforced at compile time |
| Error handling | Mix of return codes, exceptions, signals | Typed `Result<T, MumbleError>` throughout |
| Integer overflow | Possible (UB in some cases) | Checked by default in debug; wrapping in release |
| Undefined behaviour | Possible | Prevented by Rust's safety guarantees (unsafe-free) |

---

## Performance

The Rust library does not yet include audio processing, so a direct voice
latency comparison is not meaningful.  For the control-channel path:

- **C++ client**: Qt's event loop adds some overhead; socket I/O is buffered
  by `QSslSocket`.
- **Rust library**: `tokio` uses epoll/kqueue; `BufStream` provides efficient
  buffering.  The library adds essentially zero overhead beyond the protocol
  itself.

---

## Testing

| Approach | C++ | Rust |
|----------|-----|------|
| Unit tests | Qt Test framework | `cargo test` / `#[test]` |
| Integration tests | Requires a running Mumble server or mock | In-process mock server (no external dependencies) |
| CI | GitHub Actions (`cmake` build + ctest) | `cargo test` (same CI) |

The Rust integration tests spin up an in-process TLS mock server for each test
case, making them fully self-contained and fast.

---

## Migration path

The Rust library is intentionally designed as a standalone library, not a port
of the C++ GUI client.  The intended use cases are:

1. **Bots and automation** – lightweight programs that connect to Mumble servers
   without a GUI.
2. **Server-side tooling** – utilities that query or manage Mumble servers.
3. **Protocol testing** – a clean, testable implementation of the Mumble
   protocol useful for validating server behaviour.

Full feature parity with the C++ GUI client (audio, plugins, overlay) is out of
scope.  See `docs/todo.md` for the planned feature roadmap.
