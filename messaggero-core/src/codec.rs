use serde::{de::DeserializeOwned, Serialize};

use crate::CodecError;

/// Wire encoding format.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// JSON — used for A2A interoperability.
    Json,
    /// Bincode — used for the high-performance fast path.
    Binary,
}

/// Encode a value into bytes using the specified encoding.
pub fn encode<T: Serialize>(value: &T, encoding: Encoding) -> Result<Vec<u8>, CodecError> {
    match encoding {
        Encoding::Json => serde_json::to_vec(value).map_err(CodecError::Json),
        Encoding::Binary => bincode::serialize(value).map_err(CodecError::Bincode),
    }
}

/// Decode bytes into a value using the specified encoding.
pub fn decode<T: DeserializeOwned>(data: &[u8], encoding: Encoding) -> Result<T, CodecError> {
    match encoding {
        Encoding::Json => serde_json::from_slice(data).map_err(CodecError::Json),
        Encoding::Binary => bincode::deserialize(data).map_err(CodecError::Bincode),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, TaskRequest};

    #[test]
    fn roundtrip_json() {
        let req = TaskRequest::new(Message::user("hello"));
        let bytes = encode(&req, Encoding::Json).unwrap();
        let decoded: TaskRequest = decode(&bytes, Encoding::Json).unwrap();
        assert_eq!(decoded.message.text_content(), Some("hello"));
    }

    #[test]
    fn roundtrip_binary() {
        let req = TaskRequest::new(Message::user("hello"));
        let bytes = encode(&req, Encoding::Binary).unwrap();
        let decoded: TaskRequest = decode(&bytes, Encoding::Binary).unwrap();
        assert_eq!(decoded.message.text_content(), Some("hello"));
    }

    #[test]
    fn binary_is_smaller_than_json() {
        let req = TaskRequest::new(Message::user("a]longer message for comparison"));
        let json_bytes = encode(&req, Encoding::Json).unwrap();
        let bin_bytes = encode(&req, Encoding::Binary).unwrap();
        assert!(bin_bytes.len() < json_bytes.len());
    }
}
