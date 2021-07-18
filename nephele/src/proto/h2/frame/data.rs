use bytes::{Buf, BufMut, Bytes};
use std::fmt;

use crate::proto::h2::frame::{util, Error, Frame, Head, Kind, StreamId};

#[derive(Eq, PartialEq)]
pub struct Data<T = Bytes> {
    stream_id: StreamId,
    data: T,
    flags: DataFlags,
    pad_len: Option<u8>,
}

#[derive(Copy, Clone, Eq, PartialEq)]
struct DataFlags(u8);

const END_STREAM: u8 = 0x1;
const PADDED: u8 = 0x8;
const ALL: u8 = END_STREAM | PADDED;

impl<T> Data<T> {
    pub fn new(stream_id: StreamId, payload: T) -> Self {
        assert!(!stream_id.is_zero());

        Data {
            stream_id,
            data: payload,
            flags: DataFlags::default(),
            pad_len: None,
        }
    }

    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    pub fn is_end_stream(&self) -> bool {
        self.flags.is_end_stream()
    }

    pub fn set_end_stream(&mut self, val: bool) {
        if val {
            self.flags.set_end_stream();
        } else {
            self.flags.unset_end_stream();
        }
    }

    #[cfg(feature = "unstable")]
    pub fn is_padded(&self) -> bool {
        self.flags.is_padded()
    }

    #[cfg(feature = "unstable")]
    pub fn set_padded(&mut self) {
        self.flags.set_padded();
    }

    pub fn payload(&self) -> &T {
        &self.data
    }

    pub fn payload_mut(&mut self) -> &mut T {
        &mut self.data
    }

    pub fn into_payload(self) -> T {
        self.data
    }

    pub(crate) fn head(&self) -> Head {
        Head::new(Kind::Data, self.flags.into(), self.stream_id)
    }

    pub(crate) fn map<F, U>(self, f: F) -> Data<U>
    where
        F: FnOnce(T) -> U,
    {
        Data {
            stream_id: self.stream_id,
            data: f(self.data),
            flags: self.flags,
            pad_len: self.pad_len,
        }
    }
}

impl Data<Bytes> {
    pub(crate) fn load(head: Head, mut payload: Bytes) -> Result<Self, Error> {
        let flags = DataFlags::load(head.flag());

        if head.stream_id().is_zero() {
            return Err(Error::InvalidStreamId);
        }

        let pad_len = if flags.is_padded() {
            let len = util::strip_padding(&mut payload)?;
            Some(len)
        } else {
            None
        };

        Ok(Data {
            stream_id: head.stream_id(),
            data: payload,
            flags,
            pad_len,
        })
    }
}

impl<T: Buf> Data<T> {
    pub(crate) fn encode_chunk<U: BufMut>(&mut self, dst: &mut U) {
        let len = self.data.remaining() as usize;

        assert!(dst.remaining_mut() >= len);

        self.head().encode(len, dst);
        dst.put(&mut self.data);
    }
}

impl<T> From<Data<T>> for Frame<T> {
    fn from(src: Data<T>) -> Self {
        Frame::Data(src)
    }
}

impl<T> fmt::Debug for Data<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        let mut f = fmt.debug_struct("Data");
        f.field("stream_id", &self.stream_id);
        if !self.flags.is_empty() {
            f.field("flags", &self.flags);
        }
        if let Some(ref pad_len) = self.pad_len {
            f.field("pad_len", pad_len);
        }
        f.finish()
    }
}

impl DataFlags {
    fn load(bits: u8) -> DataFlags {
        DataFlags(bits & ALL)
    }

    fn is_empty(&self) -> bool {
        self.0 == 0
    }

    fn is_end_stream(&self) -> bool {
        self.0 & END_STREAM == END_STREAM
    }

    fn set_end_stream(&mut self) {
        self.0 |= END_STREAM
    }

    fn unset_end_stream(&mut self) {
        self.0 &= !END_STREAM
    }

    fn is_padded(&self) -> bool {
        self.0 & PADDED == PADDED
    }

    #[cfg(feature = "unstable")]
    fn set_padded(&mut self) {
        self.0 |= PADDED
    }
}

impl Default for DataFlags {
    fn default() -> Self {
        DataFlags(0)
    }
}

impl From<DataFlags> for u8 {
    fn from(src: DataFlags) -> u8 {
        src.0
    }
}

impl fmt::Debug for DataFlags {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        util::debug_flags(fmt, self.0)
            .flag_if(self.is_end_stream(), "END_STREAM")
            .flag_if(self.is_padded(), "PADDED")
            .finish()
    }
}
