use bytes::{Buf, BufMut, Bytes, BytesMut};
use cynthia::future::swap::{AsyncRead, AsyncWrite};
use std::error::Error as StdError;
use std::io::{self, Cursor};
use std::{cmp, fmt};

use crate::common::codec::{Decoder, Encoder, Framed, FramedRead, FramedWrite};

#[derive(Debug, Clone, Copy)]
pub struct Builder {
    max_frame_len: usize,
    length_field_len: usize,
    length_field_offset: usize,
    length_adjustment: isize,
    num_skip: Option<usize>,
    length_field_is_big_endian: bool,
}

pub struct LengthDelimitedCodecError {
    _priv: (),
}

#[derive(Debug)]
pub struct LengthDelimitedCodec {
    builder: Builder,
    state: DecodeState,
}

#[derive(Debug, Clone, Copy)]
enum DecodeState {
    Head,
    Data(usize),
}

impl LengthDelimitedCodec {
    pub fn new() -> Self {
        Self {
            builder: Builder::new(),
            state: DecodeState::Head,
        }
    }

    pub fn builder() -> Builder {
        Builder::new()
    }

    pub fn max_frame_length(&self) -> usize {
        self.builder.max_frame_len
    }

    pub fn set_max_frame_length(&mut self, val: usize) {
        self.builder.max_frame_length(val);
    }

    fn decode_head(&mut self, src: &mut BytesMut) -> io::Result<Option<usize>> {
        let head_len = self.builder.num_head_bytes();
        let field_len = self.builder.length_field_len;

        if src.len() < head_len {
            return Ok(None);
        }

        let n = {
            let mut src = Cursor::new(&mut *src);

            src.advance(self.builder.length_field_offset);

            let n = if self.builder.length_field_is_big_endian {
                src.get_uint(field_len)
            } else {
                src.get_uint_le(field_len)
            };

            if n > self.builder.max_frame_len as u64 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    LengthDelimitedCodecError { _priv: () },
                ));
            }

            let n = n as usize;

            let n = if self.builder.length_adjustment < 0 {
                n.checked_sub(-self.builder.length_adjustment as usize)
            } else {
                n.checked_add(self.builder.length_adjustment as usize)
            };

            match n {
                Some(n) => n,
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "provided length would overflow after adjustment",
                    ));
                }
            }
        };

        let num_skip = self.builder.get_num_skip();

        if num_skip > 0 {
            src.advance(num_skip);
        }

        src.reserve(n);

        Ok(Some(n))
    }

    fn decode_data(&self, n: usize, src: &mut BytesMut) -> io::Result<Option<BytesMut>> {
        if src.len() < n {
            return Ok(None);
        }

        Ok(Some(src.split_to(n)))
    }
}

impl Decoder for LengthDelimitedCodec {
    type Item = BytesMut;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<BytesMut>> {
        let n = match self.state {
            DecodeState::Head => match self.decode_head(src)? {
                Some(n) => {
                    self.state = DecodeState::Data(n);
                    n
                }
                None => return Ok(None),
            },
            DecodeState::Data(n) => n,
        };

        match self.decode_data(n, src)? {
            Some(data) => {
                self.state = DecodeState::Head;

                src.reserve(self.builder.num_head_bytes());

                Ok(Some(data))
            }
            None => Ok(None),
        }
    }
}

impl Encoder<Bytes> for LengthDelimitedCodec {
    type Error = io::Error;

    fn encode(&mut self, data: Bytes, dst: &mut BytesMut) -> Result<(), io::Error> {
        let n = data.len();

        if n > self.builder.max_frame_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                LengthDelimitedCodecError { _priv: () },
            ));
        }

        let n = if self.builder.length_adjustment < 0 {
            n.checked_add(-self.builder.length_adjustment as usize)
        } else {
            n.checked_sub(self.builder.length_adjustment as usize)
        };

        let n = n.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "provided length would overflow after adjustment",
            )
        })?;

        dst.reserve(self.builder.length_field_len + n);

        if self.builder.length_field_is_big_endian {
            dst.put_uint(n as u64, self.builder.length_field_len);
        } else {
            dst.put_uint_le(n as u64, self.builder.length_field_len);
        }

        dst.extend_from_slice(&data[..]);

        Ok(())
    }
}

impl Default for LengthDelimitedCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    pub fn new() -> Builder {
        Builder {
            max_frame_len: 8 * 1_024 * 1_024,

            length_field_len: 4,

            length_field_offset: 0,

            length_adjustment: 0,

            num_skip: None,

            length_field_is_big_endian: true,
        }
    }

    pub fn big_endian(&mut self) -> &mut Self {
        self.length_field_is_big_endian = true;
        self
    }

    pub fn little_endian(&mut self) -> &mut Self {
        self.length_field_is_big_endian = false;
        self
    }

    pub fn native_endian(&mut self) -> &mut Self {
        if cfg!(target_endian = "big") {
            self.big_endian()
        } else {
            self.little_endian()
        }
    }

    pub fn max_frame_length(&mut self, val: usize) -> &mut Self {
        self.max_frame_len = val;
        self
    }

    pub fn length_field_length(&mut self, val: usize) -> &mut Self {
        assert!(val > 0 && val <= 8, "invalid length field length");
        self.length_field_len = val;
        self
    }

    pub fn length_field_offset(&mut self, val: usize) -> &mut Self {
        self.length_field_offset = val;
        self
    }

    pub fn length_adjustment(&mut self, val: isize) -> &mut Self {
        self.length_adjustment = val;
        self
    }

    pub fn num_skip(&mut self, val: usize) -> &mut Self {
        self.num_skip = Some(val);
        self
    }

    pub fn new_codec(&self) -> LengthDelimitedCodec {
        LengthDelimitedCodec {
            builder: *self,
            state: DecodeState::Head,
        }
    }

    pub fn new_read<T>(&self, upstream: T) -> FramedRead<T, LengthDelimitedCodec>
    where
        T: AsyncRead,
    {
        FramedRead::new(upstream, self.new_codec())
    }

    pub fn new_write<T>(&self, inner: T) -> FramedWrite<T, LengthDelimitedCodec>
    where
        T: AsyncWrite,
    {
        FramedWrite::new(inner, self.new_codec())
    }

    pub fn new_framed<T>(&self, inner: T) -> Framed<T, LengthDelimitedCodec>
    where
        T: AsyncRead + AsyncWrite,
    {
        Framed::new(inner, self.new_codec())
    }

    fn num_head_bytes(&self) -> usize {
        let num = self.length_field_offset + self.length_field_len;
        cmp::max(num, self.num_skip.unwrap_or(0))
    }

    fn get_num_skip(&self) -> usize {
        self.num_skip
            .unwrap_or(self.length_field_offset + self.length_field_len)
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for LengthDelimitedCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LengthDelimitedCodecError").finish()
    }
}

impl fmt::Display for LengthDelimitedCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("frame size too big")
    }
}

impl StdError for LengthDelimitedCodecError {}
