use thiserror::Error;

/// Errors that can occur when using the Mumble client library.
#[derive(Debug, Error)]
pub enum MumbleError {
    /// An I/O error occurred on the underlying TCP or TLS stream.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A TLS handshake or certificate error.
    #[error("TLS error: {0}")]
    Tls(#[from] rustls::Error),

    /// A protocol buffer encode/decode error.
    #[error("Protobuf decode error: {0}")]
    ProtobufDecode(#[from] prost::DecodeError),

    /// A protocol buffer encode error.
    #[error("Protobuf encode error: {0}")]
    ProtobufEncode(#[from] prost::EncodeError),

    /// An unknown or unexpected message type was received.
    #[error("Unknown message type: {0}")]
    UnknownMessageType(u16),

    /// The server rejected the connection.
    #[error("Server rejected connection: {0}")]
    Rejected(String),

    /// The connection was closed before the operation completed.
    #[error("Connection closed unexpectedly")]
    ConnectionClosed,

    /// An invalid server address or DNS name was supplied.
    #[error("Invalid server address: {0}")]
    InvalidAddress(String),

    /// The server sync was not received before a timeout or disconnection.
    #[error("Server sync not received")]
    SyncTimeout,
}

pub type Result<T> = std::result::Result<T, MumbleError>;
