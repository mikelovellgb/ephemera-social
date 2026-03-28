//! Encode and decode protocol envelopes to/from bytes.
//!
//! Uses serde + JSON as the initial serialization format. This will be
//! migrated to protobuf (prost) once `.proto` files are set up. The wire
//! format is length-prefixed: a 4-byte big-endian u32 followed by the
//! serialized envelope bytes.

use crate::envelope::ProtocolEnvelope;
use bytes::{Buf, BufMut, BytesMut};

/// Maximum frame size on the wire: 1 MiB.
pub const MAX_FRAME_SIZE: u32 = 1_048_576;

/// Errors that can occur during encoding or decoding.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    /// The frame exceeds the maximum allowed size.
    #[error("frame too large: {size} bytes (max {max})")]
    FrameTooLarge {
        /// Actual size.
        size: u32,
        /// Maximum allowed.
        max: u32,
    },
    /// Not enough data available to decode a complete frame.
    #[error("incomplete frame: need {needed} more bytes")]
    Incomplete {
        /// How many additional bytes are needed.
        needed: usize,
    },
    /// Serialization or deserialization failed.
    #[error("serialization error: {0}")]
    Serialization(String),
}

/// Encode a [`ProtocolEnvelope`] into a length-prefixed byte buffer.
///
/// Wire format: `[u32 BE length][JSON bytes]`
pub fn encode(envelope: &ProtocolEnvelope, dst: &mut BytesMut) -> Result<(), CodecError> {
    let payload =
        serde_json::to_vec(envelope).map_err(|e| CodecError::Serialization(e.to_string()))?;
    let len = payload.len() as u32;
    if len > MAX_FRAME_SIZE {
        return Err(CodecError::FrameTooLarge {
            size: len,
            max: MAX_FRAME_SIZE,
        });
    }
    dst.put_u32(len);
    dst.extend_from_slice(&payload);
    Ok(())
}

/// Attempt to decode a [`ProtocolEnvelope`] from a length-prefixed byte buffer.
///
/// Returns `Ok(Some(envelope))` if a complete frame is available, `Ok(None)`
/// if more data is needed, or `Err` on protocol violations.
pub fn decode(src: &mut BytesMut) -> Result<Option<ProtocolEnvelope>, CodecError> {
    if src.len() < 4 {
        return Ok(None);
    }

    // Peek at the length without consuming.
    let len = u32::from_be_bytes([src[0], src[1], src[2], src[3]]);

    if len > MAX_FRAME_SIZE {
        return Err(CodecError::FrameTooLarge {
            size: len,
            max: MAX_FRAME_SIZE,
        });
    }

    let total = 4 + len as usize;
    if src.len() < total {
        return Ok(None);
    }

    // Consume the length prefix.
    src.advance(4);
    let payload = src.split_to(len as usize);

    let envelope: ProtocolEnvelope =
        serde_json::from_slice(&payload).map_err(|e| CodecError::Serialization(e.to_string()))?;

    Ok(Some(envelope))
}

/// Encode a [`ProtocolEnvelope`] into a standalone `Vec<u8>` (no length prefix).
///
/// Useful for signing or hashing the envelope content.
pub fn encode_bare(envelope: &ProtocolEnvelope) -> Result<Vec<u8>, CodecError> {
    serde_json::to_vec(envelope).map_err(|e| CodecError::Serialization(e.to_string()))
}

/// Decode a [`ProtocolEnvelope`] from raw bytes (no length prefix).
pub fn decode_bare(data: &[u8]) -> Result<ProtocolEnvelope, CodecError> {
    serde_json::from_slice(data).map_err(|e| CodecError::Serialization(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::MessageType;

    fn sample_envelope() -> ProtocolEnvelope {
        ProtocolEnvelope::new([0xAA; 32], MessageType::Ping, vec![1, 2, 3])
    }

    #[test]
    fn encode_decode_round_trip() {
        let env = sample_envelope();
        let mut buf = BytesMut::new();
        encode(&env, &mut buf).unwrap();

        let decoded = decode(&mut buf).unwrap().expect("should decode");
        assert_eq!(decoded.sender_node_id, env.sender_node_id);
        assert_eq!(decoded.payload_type, env.payload_type);
        assert_eq!(decoded.payload_bytes, env.payload_bytes);
    }

    #[test]
    fn decode_incomplete() {
        let mut buf = BytesMut::from(&[0u8, 0, 0, 10][..]); // says 10 bytes but only 4
        let result = decode(&mut buf).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn decode_too_short_for_length() {
        let mut buf = BytesMut::from(&[0u8, 0][..]);
        let result = decode(&mut buf).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn bare_round_trip() {
        let env = sample_envelope();
        let bytes = encode_bare(&env).unwrap();
        let decoded = decode_bare(&bytes).unwrap();
        assert_eq!(decoded.payload_type, env.payload_type);
    }

    #[test]
    fn reject_oversized_frame() {
        let mut buf = BytesMut::new();
        buf.put_u32(MAX_FRAME_SIZE + 1);
        let err = decode(&mut buf).unwrap_err();
        assert!(matches!(err, CodecError::FrameTooLarge { .. }));
    }
}
