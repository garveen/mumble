# Rust Mumble Client – TODO

This document tracks what is implemented, what is planned, and what is
deliberately out of scope for the Rust client library.

---

## ✅ Implemented

### Core protocol
- [x] TCP framing: 2-byte type + 4-byte length prefix (big-endian)
- [x] TLS/TCP connection via `tokio-rustls`
- [x] All 26 message types defined as `MessageType` enum
- [x] Generic `Frame::encode` / `Frame::decode_as` helpers
- [x] Async `read_frame` / `write_frame` using `tokio` I/O

### Handshake & synchronisation
- [x] Version exchange (both legacy v1 and new v2 encoding)
- [x] Authentication (username, password, tokens, Opus flag, client type)
- [x] CryptSetup consumption (keys stored for future UDP use)
- [x] CodecVersion consumption
- [x] ChannelState accumulation into `Vec<Channel>`
- [x] UserState accumulation into `Vec<User>`
- [x] ServerSync → `ServerInfo` (session id, max bandwidth, welcome text)
- [x] `Reject` → `MumbleError::Rejected`

### Post-sync operations
- [x] `ping()` – send a Ping with current timestamp
- [x] `send_text_message(channel_ids, text)`
- [x] `move_to_channel(channel_id)`
- [x] `run_ping_loop()` – periodic keepalive (every 15 s)

### Error handling
- [x] `MumbleError` enum with variants for I/O, TLS, protobuf, and protocol errors
- [x] `Result<T>` type alias

### Configuration
- [x] `ClientConfig` builder (host, port, username, password, tokens, cert validation toggle)

### Testing
- [x] In-process mock Mumble server (TLS + full handshake)
- [x] Unit tests for protocol framing
- [x] Integration test: connect + sync
- [x] Integration test: channel list populated
- [x] Integration test: user list populated
- [x] Integration test: ping roundtrip
- [x] Integration test: text message
- [x] Integration test: move to channel
- [x] Integration test: server rejection → `MumbleError::Rejected`
- [x] Integration test: TCP frame encoding correctness

---

## 🔲 Planned (not yet implemented)

### UDP voice channel
- [ ] OCB-AES128 encryption / decryption using keys from `CryptSetup`
- [ ] UDP socket management (bind, send, receive)
- [ ] Voice packet encoding (Opus)
- [ ] Voice packet decoding (Opus)
- [ ] UDP fallback: tunnel voice over TCP using `UDPTunnel` message type

### ACL & permissions
- [ ] `ACL` message parsing and local cache
- [ ] `PermissionQuery` / `PermissionDenied` handling
- [ ] Local permission check helpers

### User and channel management
- [ ] `UserRemove` / `ChannelRemove` to prune local caches
- [ ] `UserList` retrieval
- [ ] `BanList` retrieval
- [ ] `QueryUsers` support

### Server administration
- [ ] `UserStats` request/response
- [ ] `RequestBlob` (fetch lazy comment/texture data)
- [ ] `ServerConfig` reception and storage
- [ ] `SuggestConfig` reception

### Context actions
- [ ] `ContextActionModify` / `ContextAction` support

### Voice targets
- [ ] `VoiceTarget` message for whisper / shout

### TLS certificate authentication
- [ ] Client certificate loading from PEM/DER
- [ ] Server certificate fingerprint verification

### Observability
- [ ] Structured `tracing` spans covering each handshake phase
- [ ] Per-connection metrics (packet counts, latency)

### Examples
- [ ] `examples/bot.rs` – minimal bot that connects and lists channels
- [ ] `examples/echo_bot.rs` – bot that echoes text messages back to sender

---

## 🚫 Out of scope (for this library)

- Full audio capture / playback (handled by the application layer)
- GUI
- Plugin system
- Overlay
