//! Mumble client implementation.
//!
//! [`MumbleClient`] provides a high-level interface for connecting to a Mumble
//! server, authenticating, and exchanging messages over TLS/TCP.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufStream};
use tokio::net::TcpStream;
use tokio::time;
use tokio_rustls::TlsConnector;
use tracing::{debug, info, warn};

use crate::error::{MumbleError, Result};
use crate::mumble_proto;
use crate::proto::{read_frame, write_frame, Frame, MessageType};

/// The default Mumble server TCP port.
pub const DEFAULT_PORT: u16 = 64738;

/// Keepalive ping interval (Mumble servers disconnect after 30 s without a ping).
const PING_INTERVAL: Duration = Duration::from_secs(15);

/// Configuration for connecting to a Mumble server.
///
/// See `docs/config_guide.md` for a complete description of every field.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Server hostname or IP address.
    pub host: String,
    /// Server TCP port (default: [`DEFAULT_PORT`]).
    pub port: u16,
    /// Username to authenticate with.
    pub username: String,
    /// Optional server or user password.
    pub password: Option<String>,
    /// Optional access tokens (ACL group passwords).
    pub tokens: Vec<String>,
    /// Accept self-signed / untrusted server certificates.
    /// **Do not enable in production without additional verification.**
    pub accept_invalid_certs: bool,
}

impl ClientConfig {
    /// Create a minimal config with only the required fields.
    pub fn new(host: impl Into<String>, username: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: DEFAULT_PORT,
            username: username.into(),
            password: None,
            tokens: Vec::new(),
            accept_invalid_certs: false,
        }
    }

    /// Set the port.
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Set the password.
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Add an access token.
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.tokens.push(token.into());
        self
    }

    /// Allow self-signed certificates (useful for testing against local servers).
    pub fn accept_invalid_certs(mut self) -> Self {
        self.accept_invalid_certs = true;
        self
    }
}

/// Information received from the server during the synchronisation phase.
#[derive(Debug, Clone, Default)]
pub struct ServerInfo {
    /// The session ID assigned to this client.
    pub session: u32,
    /// Maximum allowed bandwidth in bits per second.
    pub max_bandwidth: u32,
    /// Welcome message from the server.
    pub welcome_text: String,
}

/// A channel present on the server after synchronisation.
#[derive(Debug, Clone)]
pub struct Channel {
    pub channel_id: u32,
    pub parent: u32,
    pub name: String,
    pub description: String,
    pub temporary: bool,
    pub position: i32,
}

/// A user present on the server after synchronisation.
#[derive(Debug, Clone)]
pub struct User {
    pub session: u32,
    pub name: String,
    pub channel_id: u32,
    pub muted: bool,
    pub deafened: bool,
}

/// A connected Mumble client.
///
/// After [`MumbleClient::connect`] returns the client has completed the full
/// server synchronisation handshake and is ready to send/receive messages.
pub struct MumbleClient<S> {
    stream: BufStream<S>,
    pub server_info: ServerInfo,
    pub channels: Vec<Channel>,
    pub users: Vec<User>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> MumbleClient<S> {
    /// Send a protobuf message to the server.
    pub async fn send<M: prost::Message>(&mut self, msg_type: MessageType, msg: &M) -> Result<()> {
        let encoded = Frame::encode(msg_type, msg)?;
        write_frame(&mut self.stream, &encoded).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Receive the next frame from the server.
    pub async fn recv(&mut self) -> Result<Frame> {
        read_frame(&mut self.stream).await
    }

    /// Send a ping to the server.
    pub async fn ping(&mut self) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;
        let ping = mumble_proto::Ping {
            timestamp: Some(now),
            ..Default::default()
        };
        self.send(MessageType::Ping, &ping).await
    }

    /// Send a text message to a channel.
    pub async fn send_text_message(
        &mut self,
        channel_ids: Vec<u32>,
        message: impl Into<String>,
    ) -> Result<()> {
        let msg = mumble_proto::TextMessage {
            channel_id: channel_ids,
            message: message.into(),
            ..Default::default()
        };
        self.send(MessageType::TextMessage, &msg).await
    }

    /// Move this client to a different channel.
    pub async fn move_to_channel(&mut self, channel_id: u32) -> Result<()> {
        let state = mumble_proto::UserState {
            session: Some(self.server_info.session),
            channel_id: Some(channel_id),
            ..Default::default()
        };
        self.send(MessageType::UserState, &state).await
    }

    /// Process incoming messages until a [`ServerSync`](MessageType::ServerSync) is received.
    ///
    /// Populates `self.channels` and `self.users` from the channel/user state
    /// messages the server sends before `ServerSync`.
    async fn process_until_sync(&mut self) -> Result<()> {
        loop {
            let frame = self.recv().await?;
            match frame.message_type {
                MessageType::Version => {
                    let v: mumble_proto::Version = frame.decode_as()?;
                    info!(
                        release = v.release.as_deref().unwrap_or("?"),
                        "Server version"
                    );
                }
                MessageType::CryptSetup => {
                    debug!("CryptSetup received (UDP encryption keys)");
                }
                MessageType::CodecVersion => {
                    debug!("CodecVersion received");
                }
                MessageType::ChannelState => {
                    let cs: mumble_proto::ChannelState = frame.decode_as()?;
                    let channel = Channel {
                        channel_id: cs.channel_id.unwrap_or(0),
                        parent: cs.parent.unwrap_or(0),
                        name: cs.name.unwrap_or_default(),
                        description: cs.description.unwrap_or_default(),
                        temporary: cs.temporary.unwrap_or(false),
                        position: cs.position.unwrap_or(0),
                    };
                    debug!(id = channel.channel_id, name = %channel.name, "Channel");
                    // Update existing or insert
                    if let Some(existing) = self
                        .channels
                        .iter_mut()
                        .find(|c| c.channel_id == channel.channel_id)
                    {
                        *existing = channel;
                    } else {
                        self.channels.push(channel);
                    }
                }
                MessageType::UserState => {
                    let us: mumble_proto::UserState = frame.decode_as()?;
                    let user = User {
                        session: us.session.unwrap_or(0),
                        name: us.name.unwrap_or_default(),
                        channel_id: us.channel_id.unwrap_or(0),
                        muted: us.mute.unwrap_or(false),
                        deafened: us.deaf.unwrap_or(false),
                    };
                    debug!(session = user.session, name = %user.name, "User");
                    if let Some(existing) = self
                        .users
                        .iter_mut()
                        .find(|u| u.session == user.session)
                    {
                        *existing = user;
                    } else {
                        self.users.push(user);
                    }
                }
                MessageType::ServerSync => {
                    let ss: mumble_proto::ServerSync = frame.decode_as()?;
                    self.server_info.session = ss.session.unwrap_or(0);
                    self.server_info.max_bandwidth = ss.max_bandwidth.unwrap_or(0);
                    self.server_info.welcome_text =
                        ss.welcome_text.unwrap_or_default();
                    info!(
                        session = self.server_info.session,
                        welcome = %self.server_info.welcome_text,
                        "ServerSync – ready"
                    );
                    return Ok(());
                }
                MessageType::Reject => {
                    let r: mumble_proto::Reject = frame.decode_as()?;
                    let reason = r.reason.unwrap_or_else(|| "no reason given".into());
                    return Err(MumbleError::Rejected(reason));
                }
                MessageType::PermissionDenied => {
                    let pd: mumble_proto::PermissionDenied = frame.decode_as()?;
                    warn!(reason = pd.reason.as_deref().unwrap_or("?"), "PermissionDenied");
                }
                other => {
                    debug!(?other, "Unhandled message during sync");
                }
            }
        }
    }

    /// Perform the version + authenticate handshake.
    async fn handshake(&mut self, config: &ClientConfig) -> Result<()> {
        // Send our Version first.
        let version = mumble_proto::Version {
            version_v1: Some(make_version_v1(1, 5, 0)),
            version_v2: Some(make_version_v2(1, 5, 0, 0)),
            release: Some("mumble-rust-client/0.1.0".into()),
            os: Some(std::env::consts::OS.into()),
            os_version: Some("unknown".into()),
        };
        self.send(MessageType::Version, &version).await?;

        // Then Authenticate.
        let auth = mumble_proto::Authenticate {
            username: Some(config.username.clone()),
            password: config.password.clone(),
            tokens: config.tokens.clone(),
            opus: Some(true),
            client_type: Some(1), // BOT
            ..Default::default()
        };
        self.send(MessageType::Authenticate, &auth).await?;
        Ok(())
    }

    /// Keep the connection alive by sending periodic pings.
    ///
    /// Call this in a separate task or after each set of operations.  Returns
    /// once the first `Result::Err` is encountered (connection lost).
    pub async fn run_ping_loop(&mut self) -> Result<()> {
        loop {
            time::sleep(PING_INTERVAL).await;
            self.ping().await?;
        }
    }
}

impl MumbleClient<tokio_rustls::client::TlsStream<TcpStream>> {
    /// Connect to a Mumble server using TLS over TCP.
    ///
    /// This completes the full Mumble handshake (version exchange, authentication,
    /// channel/user sync, ServerSync) before returning.
    pub async fn connect(config: ClientConfig) -> Result<Self> {
        let addr = format!("{}:{}", config.host, config.port);
        let tcp = TcpStream::connect(&addr).await?;
        tcp.set_nodelay(true)?;

        let tls_config = build_tls_config(config.accept_invalid_certs)?;
        let connector = TlsConnector::from(Arc::new(tls_config));
        let domain = rustls::pki_types::ServerName::try_from(config.host.clone())
            .map_err(|_| MumbleError::InvalidAddress(config.host.clone()))?;
        let tls_stream = connector.connect(domain.to_owned(), tcp).await?;

        let mut client = MumbleClient {
            stream: BufStream::new(tls_stream),
            server_info: ServerInfo::default(),
            channels: Vec::new(),
            users: Vec::new(),
        };

        client.handshake(&config).await?;
        client.process_until_sync().await?;
        Ok(client)
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Build a legacy v1 version word: major(16-bit) | minor(8-bit) | patch(8-bit).
fn make_version_v1(major: u32, minor: u32, patch: u32) -> u32 {
    (major << 16) | (minor << 8) | patch
}

/// Build a v2 version word as defined in Mumble.proto comment.
fn make_version_v2(major: u64, minor: u64, patch: u64, build: u64) -> u64 {
    (major << 48) | (minor << 32) | (patch << 16) | build
}

/// Build a `rustls` client config.
fn build_tls_config(accept_invalid: bool) -> Result<rustls::ClientConfig> {
    if accept_invalid {
        // Accept any certificate – useful for testing against local/self-signed servers.
        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCertVerifier))
            .with_no_client_auth();
        Ok(config)
    } else {
        let mut root_store = rustls::RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        Ok(config)
    }
}

/// A no-op certificate verifier used when `accept_invalid_certs` is set.
///
/// # Security
/// This disables all TLS certificate verification.  Only use for local
/// testing against self-signed or untrusted certificates.
#[derive(Debug)]
struct NoCertVerifier;

impl rustls::client::danger::ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_v1_encoding() {
        assert_eq!(make_version_v1(1, 2, 0), 0x0001_0200);
        assert_eq!(make_version_v1(1, 5, 0), 0x0001_0500);
    }

    #[test]
    fn test_version_v2_encoding() {
        let v = make_version_v2(1, 5, 0, 0);
        let major = (v >> 48) & 0xFFFF;
        let minor = (v >> 32) & 0xFFFF;
        let patch = (v >> 16) & 0xFFFF;
        assert_eq!(major, 1);
        assert_eq!(minor, 5);
        assert_eq!(patch, 0);
    }

    #[test]
    fn test_client_config_builder() {
        let cfg = ClientConfig::new("localhost", "bot")
            .with_port(64738)
            .with_password("secret")
            .with_token("vip")
            .accept_invalid_certs();

        assert_eq!(cfg.host, "localhost");
        assert_eq!(cfg.port, 64738);
        assert_eq!(cfg.username, "bot");
        assert_eq!(cfg.password.as_deref(), Some("secret"));
        assert_eq!(cfg.tokens, vec!["vip"]);
        assert!(cfg.accept_invalid_certs);
    }
}
