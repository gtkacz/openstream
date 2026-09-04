//! Control streams carry length-prefixed postcard messages.

use crate::NetError;
use iroh::endpoint::{RecvStream, SendStream};
use serde::{Serialize, de::DeserializeOwned};
pub fn encode_framed<T: Serialize>(msg: &T) -> Result<Vec<u8>, NetError> {
    let body = brp_proto::encode(msg)?;
    let len =
        u32::try_from(body.len()).map_err(|_| NetError::Protocol("control message too large"))?;
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}
pub async fn write_msg<T: Serialize>(s: &mut SendStream, msg: &T) -> Result<(), NetError> {
    s.write_all(&encode_framed(msg)?)
        .await
        .map_err(NetError::stream)
}
pub async fn read_msg<T: DeserializeOwned>(s: &mut RecvStream, max: usize) -> Result<T, NetError> {
    let mut len = [0; 4];
    s.read_exact(&mut len).await.map_err(NetError::stream)?;
    let n = u32::from_le_bytes(len) as usize;
    if n > max {
        return Err(NetError::Protocol("control message exceeds the size limit"));
    }
    let mut body = vec![0; n];
    s.read_exact(&mut body).await.map_err(NetError::stream)?;
    Ok(brp_proto::decode(&body)?)
}

#[cfg(test)]
mod tests {
    use brp_proto::ViewerMessage;

    use super::*;

    #[test]
    fn framed_message_is_length_prefixed_little_endian() {
        let msg = ViewerMessage::Subscribe {
            live_id: 1,
            preset_id: 1,
            want_audio: false,
        };
        let framed = encode_framed(&msg).unwrap();
        let body = brp_proto::encode(&msg).unwrap();
        assert_eq!(&framed[..4], &(body.len() as u32).to_le_bytes());
        assert_eq!(&framed[4..], &body[..]);
    }
}
