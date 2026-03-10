//! Integration tests for the mumble-client library.
//!
//! These tests use an in-process mock Mumble server to exercise the full
//! TCP framing, handshake, and message-exchange code paths without requiring
//! a real Mumble server to be running.

use std::sync::Arc;

use mumble_client::mumble_proto;
use mumble_client::proto::{read_frame, write_frame, Frame, MessageType};
use mumble_client::{ClientConfig, MumbleClient};
use prost::Message;
use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig;
use tokio::io::BufStream;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

// ── helpers ──────────────────────────────────────────────────────────────────

/// Spawn a minimal mock Mumble server on an ephemeral port.
///
/// The server performs the full Mumble handshake:
/// 1. TLS accept
/// 2. Read Version from client
/// 3. Send Version back
/// 4. Read Authenticate
/// 5. Send CryptSetup, CodecVersion
/// 6. Send ChannelState (root), ChannelState (general)
/// 7. Send UserState (server user + connecting user)
/// 8. Send ServerSync
///
/// After sync, the server echoes ping responses and can receive text messages.
struct MockServer {
    addr: std::net::SocketAddr,
    #[allow(dead_code)]
    acceptor: TlsAcceptor,
}

impl MockServer {
    async fn start() -> Self {
        // Generate a self-signed certificate for the mock server.
        let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
        let cert = generate_simple_self_signed(subject_alt_names).unwrap();
        let cert_der = CertificateDer::from(cert.cert.der().to_vec());
        let key_der = PrivateKeyDer::try_from(cert.key_pair.serialize_der()).unwrap();

        let server_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(server_config));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let acceptor_clone = acceptor.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let acceptor = acceptor_clone.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(acceptor, stream).await {
                        // Only log non-EOF errors; clients closing the connection is normal.
                        if !matches!(e, mumble_client::MumbleError::ConnectionClosed) {
                            eprintln!("Mock server error: {e}");
                        }
                    }
                });
            }
        });

        Self { addr, acceptor }
    }
}

/// Handle a single mock-server connection through the full Mumble handshake.
async fn handle_connection(
    acceptor: TlsAcceptor,
    stream: TcpStream,
) -> mumble_client::Result<()> {
    let tls = acceptor.accept(stream).await?;
    let mut buf = BufStream::new(tls);

    // — Read client Version —
    let frame = read_frame(&mut buf).await?;
    assert_eq!(frame.message_type, MessageType::Version);

    // — Send server Version —
    let sv = mumble_proto::Version {
        version_v1: Some((1 << 16) | (5 << 8)),
        release: Some("mock-server/1.0".into()),
        os: Some("Linux".into()),
        os_version: Some("test".into()),
        ..Default::default()
    };
    let encoded = Frame::encode(MessageType::Version, &sv)?;
    write_frame(&mut buf, &encoded).await?;
    tokio::io::AsyncWriteExt::flush(&mut buf).await?;

    // — Read Authenticate —
    let frame = read_frame(&mut buf).await?;
    assert_eq!(frame.message_type, MessageType::Authenticate);
    let auth: mumble_proto::Authenticate = frame.decode_as()?;
    let username = auth.username.clone().unwrap_or_else(|| "unknown".into());

    // — Send CryptSetup —
    let crypt = mumble_proto::CryptSetup {
        key: Some(vec![0u8; 16]),
        client_nonce: Some(vec![0u8; 16]),
        server_nonce: Some(vec![0u8; 16]),
    };
    let encoded = Frame::encode(MessageType::CryptSetup, &crypt)?;
    write_frame(&mut buf, &encoded).await?;

    // — Send CodecVersion —
    let codec = mumble_proto::CodecVersion {
        alpha: -2147483637,
        beta: 0,
        prefer_alpha: false,
        opus: Some(true),
    };
    let encoded = Frame::encode(MessageType::CodecVersion, &codec)?;
    write_frame(&mut buf, &encoded).await?;

    // — Send ChannelState (root) —
    let root = mumble_proto::ChannelState {
        channel_id: Some(0),
        name: Some("Root".into()),
        parent: Some(0),
        ..Default::default()
    };
    let encoded = Frame::encode(MessageType::ChannelState, &root)?;
    write_frame(&mut buf, &encoded).await?;

    // — Send ChannelState (general) —
    let general = mumble_proto::ChannelState {
        channel_id: Some(1),
        parent: Some(0),
        name: Some("General".into()),
        position: Some(0),
        ..Default::default()
    };
    let encoded = Frame::encode(MessageType::ChannelState, &general)?;
    write_frame(&mut buf, &encoded).await?;

    // — Send UserState for a pre-existing user —
    let existing_user = mumble_proto::UserState {
        session: Some(1),
        name: Some("ExistingUser".into()),
        channel_id: Some(1),
        ..Default::default()
    };
    let encoded = Frame::encode(MessageType::UserState, &existing_user)?;
    write_frame(&mut buf, &encoded).await?;

    // — Send UserState for the connecting user (session=42) —
    let connecting_user = mumble_proto::UserState {
        session: Some(42),
        name: Some(username),
        channel_id: Some(0),
        ..Default::default()
    };
    let encoded = Frame::encode(MessageType::UserState, &connecting_user)?;
    write_frame(&mut buf, &encoded).await?;

    // — Send ServerSync —
    let sync = mumble_proto::ServerSync {
        session: Some(42),
        max_bandwidth: Some(72000),
        welcome_text: Some("Welcome to the mock server!".into()),
        permissions: Some(0),
    };
    let encoded = Frame::encode(MessageType::ServerSync, &sync)?;
    write_frame(&mut buf, &encoded).await?;
    tokio::io::AsyncWriteExt::flush(&mut buf).await?;

    // — Post-sync: echo pings and handle text messages —
    loop {
        let frame = match read_frame(&mut buf).await {
            Ok(f) => f,
            Err(mumble_client::MumbleError::ConnectionClosed) => return Ok(()),
            Err(e) => return Err(e),
        };

        match frame.message_type {
            MessageType::Ping => {
                let ping: mumble_proto::Ping = frame.decode_as()?;
                let encoded = Frame::encode(MessageType::Ping, &ping)?;
                write_frame(&mut buf, &encoded).await?;
                tokio::io::AsyncWriteExt::flush(&mut buf).await?;
            }
            MessageType::UserState => {
                // Acknowledge a channel move by echoing back.
                let encoded = Frame::encode(MessageType::UserState, &mumble_proto::UserState {
                    session: Some(42),
                    channel_id: Some(1),
                    ..Default::default()
                })?;
                write_frame(&mut buf, &encoded).await?;
                tokio::io::AsyncWriteExt::flush(&mut buf).await?;
            }
            MessageType::TextMessage => {
                // Just consume, no echo needed for tests.
            }
            _ => {}
        }
    }
}

/// Build a [`ClientConfig`] pointing at the mock server, with cert verification disabled.
fn mock_config(addr: std::net::SocketAddr, username: &str) -> ClientConfig {
    ClientConfig::new("localhost", username)
        .with_port(addr.port())
        .accept_invalid_certs()
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// Verify that the client successfully connects and completes the Mumble
/// synchronisation handshake against the mock server.
#[tokio::test]
async fn test_connect_and_sync() {
    let server = MockServer::start().await;
    let config = mock_config(server.addr, "TestBot");

    let client = MumbleClient::connect(config)
        .await
        .expect("connect should succeed");

    // ServerSync should have given us session=42.
    assert_eq!(client.server_info.session, 42);
    assert_eq!(client.server_info.max_bandwidth, 72000);
    assert_eq!(client.server_info.welcome_text, "Welcome to the mock server!");
}

/// Verify that the channel list is populated after sync.
#[tokio::test]
async fn test_channels_populated() {
    let server = MockServer::start().await;
    let config = mock_config(server.addr, "ChannelBot");

    let client = MumbleClient::connect(config).await.unwrap();

    // The mock server sends Root (id=0) and General (id=1).
    assert_eq!(client.channels.len(), 2);
    let root = client.channels.iter().find(|c| c.channel_id == 0).unwrap();
    assert_eq!(root.name, "Root");
    let general = client.channels.iter().find(|c| c.channel_id == 1).unwrap();
    assert_eq!(general.name, "General");
    assert_eq!(general.parent, 0);
}

/// Verify that the user list is populated (both the pre-existing user and the
/// connecting user should appear).
#[tokio::test]
async fn test_users_populated() {
    let server = MockServer::start().await;
    let config = mock_config(server.addr, "UserBot");

    let client = MumbleClient::connect(config).await.unwrap();

    assert_eq!(client.users.len(), 2);
    let bot = client.users.iter().find(|u| u.session == 42).unwrap();
    assert_eq!(bot.name, "UserBot");
}

/// Verify that the client can send a ping and receive a response.
#[tokio::test]
async fn test_ping_roundtrip() {
    let server = MockServer::start().await;
    let config = mock_config(server.addr, "PingBot");

    let mut client = MumbleClient::connect(config).await.unwrap();

    // Send a ping.
    client.ping().await.expect("ping should succeed");

    // The mock server echoes the ping back; receive it.
    let frame = client.recv().await.expect("should receive ping response");
    assert_eq!(frame.message_type, MessageType::Ping);
    let pong: mumble_proto::Ping = frame.decode_as().unwrap();
    assert!(pong.timestamp.is_some());
}

/// Verify that the client can send a text message without error.
#[tokio::test]
async fn test_send_text_message() {
    let server = MockServer::start().await;
    let config = mock_config(server.addr, "MsgBot");

    let mut client = MumbleClient::connect(config).await.unwrap();

    client
        .send_text_message(vec![0], "Hello, world!")
        .await
        .expect("text message should succeed");
}

/// Verify that the client can request a channel move.
#[tokio::test]
async fn test_move_to_channel() {
    let server = MockServer::start().await;
    let config = mock_config(server.addr, "MoveBot");

    let mut client = MumbleClient::connect(config).await.unwrap();

    client
        .move_to_channel(1)
        .await
        .expect("move_to_channel should succeed");
}

/// Verify that a server rejection is surfaced as [`MumbleError::Rejected`].
#[tokio::test]
async fn test_server_rejection() {
    // Spawn a server that immediately rejects the client.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let subject_alt_names = vec!["localhost".to_string()];
    let cert = generate_simple_self_signed(subject_alt_names).unwrap();
    let cert_der = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivateKeyDer::try_from(cert.key_pair.serialize_der()).unwrap();
    let server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(server_config));

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let tls = acceptor.accept(stream).await.unwrap();
        let mut buf = BufStream::new(tls);

        // Read Version, skip Authenticate.
        let _ = read_frame(&mut buf).await;
        let _ = read_frame(&mut buf).await;

        // Send a Reject.
        let reject = mumble_proto::Reject {
            r#type: Some(mumble_proto::reject::RejectType::WrongUserPw as i32),
            reason: Some("Wrong password".into()),
        };
        let encoded = Frame::encode(MessageType::Reject, &reject).unwrap();
        write_frame(&mut buf, &encoded).await.unwrap();
        tokio::io::AsyncWriteExt::flush(&mut buf).await.unwrap();
    });

    let config = ClientConfig::new("localhost", "RejectBot")
        .with_port(addr.port())
        .accept_invalid_certs();

    let result = MumbleClient::connect(config).await;
    assert!(result.is_err());
    assert!(matches!(result, Err(mumble_client::MumbleError::Rejected(_))));
}

/// Verify that TCP framing (type/length header) is correct end-to-end.
#[tokio::test]
async fn test_proto_framing_correctness() {
    let ping = mumble_proto::Ping {
        timestamp: Some(0xDEAD_BEEF),
        tcp_packets: Some(5),
        ..Default::default()
    };
    let encoded = Frame::encode(MessageType::Ping, &ping).unwrap();

    // Validate header bytes.
    let msg_type = u16::from_be_bytes([encoded[0], encoded[1]]);
    let payload_len = u32::from_be_bytes([encoded[2], encoded[3], encoded[4], encoded[5]]);
    assert_eq!(msg_type, MessageType::Ping as u16);
    assert_eq!(payload_len as usize, ping.encoded_len());

    // Decode the payload.
    let payload = encoded.slice(6..);
    let frame = Frame {
        message_type: MessageType::Ping,
        payload,
    };
    let decoded: mumble_proto::Ping = frame.decode_as().unwrap();
    assert_eq!(decoded.timestamp, Some(0xDEAD_BEEF));
    assert_eq!(decoded.tcp_packets, Some(5));
}
