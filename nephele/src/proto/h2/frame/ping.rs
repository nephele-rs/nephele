use bytes::BufMut;

use crate::proto::h2::frame::{Error, Frame, Head, Kind, StreamId};

const ACK_FLAG: u8 = 0x1;

pub type Payload = [u8; 8];

#[derive(Debug, Eq, PartialEq)]
pub struct Ping {
    ack: bool,
    payload: Payload,
}

const SHUTDOWN_PAYLOAD: Payload = [0x0b, 0x7b, 0xa2, 0xf0, 0x8b, 0x9b, 0xfe, 0x54];
const USER_PAYLOAD: Payload = [0x3b, 0x7c, 0xdb, 0x7a, 0x0b, 0x87, 0x16, 0xb4];

impl Ping {
    #[cfg(feature = "unstable")]
    pub const SHUTDOWN: Payload = SHUTDOWN_PAYLOAD;

    #[cfg(not(feature = "unstable"))]
    pub(crate) const SHUTDOWN: Payload = SHUTDOWN_PAYLOAD;

    #[cfg(feature = "unstable")]
    pub const USER: Payload = USER_PAYLOAD;

    #[cfg(not(feature = "unstable"))]
    pub(crate) const USER: Payload = USER_PAYLOAD;

    pub fn new(payload: Payload) -> Ping {
        Ping {
            ack: false,
            payload,
        }
    }

    pub fn pong(payload: Payload) -> Ping {
        Ping { ack: true, payload }
    }

    pub fn is_ack(&self) -> bool {
        self.ack
    }

    pub fn payload(&self) -> &Payload {
        &self.payload
    }

    pub fn into_payload(self) -> Payload {
        self.payload
    }

    pub fn load(head: Head, bytes: &[u8]) -> Result<Ping, Error> {
        debug_assert_eq!(head.kind(), crate::proto::h2::frame::Kind::Ping);

        if !head.stream_id().is_zero() {
            return Err(Error::InvalidStreamId);
        }

        if bytes.len() != 8 {
            return Err(Error::BadFrameSize);
        }

        let mut payload = [0; 8];
        payload.copy_from_slice(bytes);

        let ack = head.flag() & ACK_FLAG != 0;

        Ok(Ping { ack, payload })
    }

    pub fn encode<B: BufMut>(&self, dst: &mut B) {
        let sz = self.payload.len();
        tracing::trace!("encoding PING; ack={} len={}", self.ack, sz);

        let flags = if self.ack { ACK_FLAG } else { 0 };
        let head = Head::new(Kind::Ping, flags, StreamId::zero());

        head.encode(sz, dst);
        dst.put_slice(&self.payload);
    }
}

impl<T> From<Ping> for Frame<T> {
    fn from(src: Ping) -> Frame<T> {
        Frame::Ping(src)
    }
}
