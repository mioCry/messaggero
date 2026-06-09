use crate::core::{CodecError, TaskRequest, TaskResponse};
use bytes::{Buf, BufMut, BytesMut};
use serde::{Deserialize, Serialize};
use tokio_util::codec::{Decoder, Encoder};

/// Wire message for the fast binary transport.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FastMessage {
    TaskRequest(TaskRequest),
    TaskResponse(TaskResponse),
    Ping,
    Pong,
    Error(String),
}

/// Length-prefixed binary codec using bincode serialization.
///
/// Wire format: `[4 bytes: payload length (big-endian u32)][N bytes: bincode payload]`
pub struct FastCodec;

const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024; // 16 MiB

impl Decoder for FastCodec {
    type Item = FastMessage;
    type Error = CodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 4 {
            return Ok(None);
        }

        let len = u32::from_be_bytes([src[0], src[1], src[2], src[3]]) as usize;

        if len > MAX_FRAME_SIZE {
            return Err(CodecError::InvalidFrame(format!(
                "frame too large: {len} bytes (max {MAX_FRAME_SIZE})"
            )));
        }

        if src.len() < 4 + len {
            src.reserve(4 + len - src.len());
            return Ok(None);
        }

        src.advance(4);
        let data = src.split_to(len);
        let msg = bincode::deserialize(&data).map_err(CodecError::Bincode)?;
        Ok(Some(msg))
    }
}

impl Encoder<FastMessage> for FastCodec {
    type Error = CodecError;

    fn encode(&mut self, item: FastMessage, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let data = bincode::serialize(&item).map_err(CodecError::Bincode)?;

        if data.len() > MAX_FRAME_SIZE {
            return Err(CodecError::InvalidFrame(format!(
                "payload too large: {} bytes (max {MAX_FRAME_SIZE})",
                data.len()
            )));
        }

        let len = u32::try_from(data.len()).map_err(|_| {
            CodecError::InvalidFrame(format!(
                "payload too large: {} bytes (max {MAX_FRAME_SIZE})",
                data.len()
            ))
        })?;
        dst.reserve(4 + data.len());
        dst.put_u32(len);
        dst.extend_from_slice(&data);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Message;

    #[test]
    fn roundtrip_through_codec() {
        let mut codec = FastCodec;
        let msg = FastMessage::TaskRequest(TaskRequest::new(Message::user("hello")));

        let mut buf = BytesMut::new();
        codec.encode(msg, &mut buf).unwrap();

        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        match decoded {
            FastMessage::TaskRequest(req) => {
                assert_eq!(req.message.text_content(), Some("hello"));
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn partial_read() {
        let mut codec = FastCodec;
        let msg = FastMessage::Ping;

        let mut buf = BytesMut::new();
        codec.encode(msg, &mut buf).unwrap();

        let full = buf.split();
        let mut partial = BytesMut::from(&full[..3]);
        assert!(codec.decode(&mut partial).unwrap().is_none());
    }
}
