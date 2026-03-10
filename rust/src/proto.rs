//! TCP message framing for the Mumble protocol.
//!
//! Each Mumble message on the TCP channel is prefixed with a 6-byte header:
//!   - 2 bytes: message type (big-endian u16)
//!   - 4 bytes: payload length in bytes (big-endian u32)
//!
//! Followed immediately by the payload bytes (a Protocol Buffers encoded message).
//!
//! Reference: <https://mumble-voip.github.io/mumble-docs/en/latest/dev/network-protocol/>

use bytes::{BufMut, Bytes, BytesMut};
use prost::Message;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::{MumbleError, Result};

/// The size of the framing header (type u16 + length u32).
const HEADER_SIZE: usize = 6;

/// Mumble TCP message type identifiers as defined in the protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum MessageType {
    Version = 0,
    UdpTunnel = 1,
    Authenticate = 2,
    Ping = 3,
    Reject = 4,
    ServerSync = 5,
    ChannelRemove = 6,
    ChannelState = 7,
    UserRemove = 8,
    UserState = 9,
    BanList = 10,
    TextMessage = 11,
    PermissionDenied = 12,
    Acl = 13,
    QueryUsers = 14,
    CryptSetup = 15,
    ContextActionModify = 16,
    ContextAction = 17,
    UserList = 18,
    VoiceTarget = 19,
    PermissionQuery = 20,
    CodecVersion = 21,
    UserStats = 22,
    RequestBlob = 23,
    ServerConfig = 24,
    SuggestConfig = 25,
}

impl TryFrom<u16> for MessageType {
    type Error = MumbleError;

    fn try_from(value: u16) -> Result<Self> {
        match value {
            0 => Ok(Self::Version),
            1 => Ok(Self::UdpTunnel),
            2 => Ok(Self::Authenticate),
            3 => Ok(Self::Ping),
            4 => Ok(Self::Reject),
            5 => Ok(Self::ServerSync),
            6 => Ok(Self::ChannelRemove),
            7 => Ok(Self::ChannelState),
            8 => Ok(Self::UserRemove),
            9 => Ok(Self::UserState),
            10 => Ok(Self::BanList),
            11 => Ok(Self::TextMessage),
            12 => Ok(Self::PermissionDenied),
            13 => Ok(Self::Acl),
            14 => Ok(Self::QueryUsers),
            15 => Ok(Self::CryptSetup),
            16 => Ok(Self::ContextActionModify),
            17 => Ok(Self::ContextAction),
            18 => Ok(Self::UserList),
            19 => Ok(Self::VoiceTarget),
            20 => Ok(Self::PermissionQuery),
            21 => Ok(Self::CodecVersion),
            22 => Ok(Self::UserStats),
            23 => Ok(Self::RequestBlob),
            24 => Ok(Self::ServerConfig),
            25 => Ok(Self::SuggestConfig),
            t => Err(MumbleError::UnknownMessageType(t)),
        }
    }
}

/// A raw Mumble TCP frame (type + payload bytes).
#[derive(Debug, Clone)]
pub struct Frame {
    pub message_type: MessageType,
    pub payload: Bytes,
}

impl Frame {
    /// Encode a protobuf message into a framed byte buffer ready to send.
    pub fn encode<M: Message>(msg_type: MessageType, msg: &M) -> Result<Bytes> {
        let payload_len = msg.encoded_len();
        let mut buf = BytesMut::with_capacity(HEADER_SIZE + payload_len);
        buf.put_u16(msg_type as u16);
        buf.put_u32(payload_len as u32);
        msg.encode(&mut buf)?;
        Ok(buf.freeze())
    }

    /// Decode the payload of this frame as a specific protobuf message type.
    pub fn decode_as<M: Message + Default>(&self) -> Result<M> {
        Ok(M::decode(self.payload.clone())?)
    }
}

/// Read a single [`Frame`] from an async reader.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Frame> {
    let mut header = [0u8; HEADER_SIZE];
    reader
        .read_exact(&mut header)
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                MumbleError::ConnectionClosed
            } else {
                MumbleError::Io(e)
            }
        })?;

    let msg_type_raw = u16::from_be_bytes([header[0], header[1]]);
    let payload_len = u32::from_be_bytes([header[2], header[3], header[4], header[5]]) as usize;

    let message_type = MessageType::try_from(msg_type_raw)?;

    let mut payload = vec![0u8; payload_len];
    reader.read_exact(&mut payload).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            MumbleError::ConnectionClosed
        } else {
            MumbleError::Io(e)
        }
    })?;

    Ok(Frame {
        message_type,
        payload: Bytes::from(payload),
    })
}

/// Write a single pre-encoded frame to an async writer.
pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, frame: &Bytes) -> Result<()> {
    writer.write_all(frame).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mumble_proto;

    #[test]
    fn test_message_type_roundtrip() {
        for t in 0u16..=25 {
            let mt = MessageType::try_from(t).expect("valid type");
            assert_eq!(mt as u16, t);
        }
    }

    #[test]
    fn test_unknown_message_type() {
        assert!(MessageType::try_from(255).is_err());
    }

    #[test]
    fn test_frame_encode_decode_ping() {
        let ping = mumble_proto::Ping {
            timestamp: Some(12345),
            ..Default::default()
        };
        let encoded = Frame::encode(MessageType::Ping, &ping).unwrap();
        // Header: type=3, length
        assert_eq!(u16::from_be_bytes([encoded[0], encoded[1]]), 3);
        let len = u32::from_be_bytes([encoded[2], encoded[3], encoded[4], encoded[5]]) as usize;
        assert_eq!(len, ping.encoded_len());

        // Decode back
        let payload = encoded.slice(HEADER_SIZE..);
        let frame = Frame {
            message_type: MessageType::Ping,
            payload,
        };
        let decoded: mumble_proto::Ping = frame.decode_as().unwrap();
        assert_eq!(decoded.timestamp, Some(12345));
    }

    #[tokio::test]
    async fn test_read_write_frame() {
        use tokio::io::duplex;

        let ping = mumble_proto::Ping {
            timestamp: Some(99),
            ..Default::default()
        };
        let encoded = Frame::encode(MessageType::Ping, &ping).unwrap();

        let (mut client, mut server) = duplex(1024);
        write_frame(&mut client, &encoded).await.unwrap();
        drop(client); // close write side

        let frame = read_frame(&mut server).await.unwrap();
        assert_eq!(frame.message_type, MessageType::Ping);
        let decoded: mumble_proto::Ping = frame.decode_as().unwrap();
        assert_eq!(decoded.timestamp, Some(99));
    }
}
