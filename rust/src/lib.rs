//! # mumble-client
//!
//! A Rust client library for the [Mumble](https://www.mumble.info) voice-chat
//! protocol.
//!
//! ## Quick start
//!
//! ```no_run
//! use mumble_client::{ClientConfig, MumbleClient};
//!
//! #[tokio::main]
//! async fn main() -> mumble_client::Result<()> {
//!     let config = ClientConfig::new("mumble.example.com", "MyBot");
//!     let mut client = MumbleClient::connect(config).await?;
//!
//!     println!("Connected! Session: {}", client.server_info.session);
//!     println!("Channels: {}", client.channels.len());
//!     println!("Users online: {}", client.users.len());
//!
//!     client.ping().await?;
//!     Ok(())
//! }
//! ```
//!
//! For more details see `docs/config_guide.md`.

pub mod client;
pub mod error;
pub mod proto;

// Re-export the generated protobuf types under a clean module name.
pub mod mumble_proto {
    include!(concat!(env!("OUT_DIR"), "/mumble_proto.rs"));
}

pub use client::{Channel, ClientConfig, MumbleClient, ServerInfo, User, DEFAULT_PORT};
pub use error::{MumbleError, Result};
pub use proto::MessageType;
