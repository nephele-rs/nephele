use bytes::{Buf, BufMut, BytesMut};
use std::{cmp, fmt, io, str, usize};

use crate::common::codec::decoder::Decoder;
use crate::common::codec::encoder::Encoder;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct LinesCodec {
    next_index: usize,

    max_length: usize,

    is_discarding: bool,
}

impl LinesCodec {
    pub fn new() -> LinesCodec {
        LinesCodec {
            next_index: 0,
            max_length: usize::MAX,
            is_discarding: false,
        }
    }

    pub fn new_with_max_length(max_length: usize) -> Self {
        LinesCodec {
            max_length,
            ..LinesCodec::new()
        }
    }

    pub fn max_length(&self) -> usize {
        self.max_length
    }
}

fn utf8(buf: &[u8]) -> Result<&str, io::Error> {
    str::from_utf8(buf)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Unable to decode input as UTF8"))
}

fn without_carriage_return(s: &[u8]) -> &[u8] {
    if let Some(&b'\r') = s.last() {
        &s[..s.len() - 1]
    } else {
        s
    }
}

impl Decoder for LinesCodec {
    type Item = String;
    type Error = LinesCodecError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<String>, LinesCodecError> {
        loop {
            let read_to = cmp::min(self.max_length.saturating_add(1), buf.len());

            let newline_offset = buf[self.next_index..read_to]
                .iter()
                .position(|b| *b == b'\n');

            match (self.is_discarding, newline_offset) {
                (true, Some(offset)) => {
                    buf.advance(offset + self.next_index + 1);
                    self.is_discarding = false;
                    self.next_index = 0;
                }
                (true, None) => {
                    buf.advance(read_to);
                    self.next_index = 0;
                    if buf.is_empty() {
                        return Err(LinesCodecError::MaxLineLengthExceeded);
                    }
                }
                (false, Some(offset)) => {
                    let newline_index = offset + self.next_index;
                    self.next_index = 0;
                    let line = buf.split_to(newline_index + 1);
                    let line = &line[..line.len() - 1];
                    let line = without_carriage_return(line);
                    let line = utf8(line)?;
                    return Ok(Some(line.to_string()));
                }
                (false, None) if buf.len() > self.max_length => {
                    self.is_discarding = true;
                    return Err(LinesCodecError::MaxLineLengthExceeded);
                }
                (false, None) => {
                    self.next_index = read_to;
                    return Ok(None);
                }
            }
        }
    }

    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<String>, LinesCodecError> {
        Ok(match self.decode(buf)? {
            Some(frame) => Some(frame),
            None => {
                if buf.is_empty() || buf == &b"\r"[..] {
                    None
                } else {
                    let line = buf.split_to(buf.len());
                    let line = without_carriage_return(&line);
                    let line = utf8(line)?;
                    self.next_index = 0;
                    Some(line.to_string())
                }
            }
        })
    }
}

impl<T> Encoder<T> for LinesCodec
where
    T: AsRef<str>,
{
    type Error = LinesCodecError;

    fn encode(&mut self, line: T, buf: &mut BytesMut) -> Result<(), LinesCodecError> {
        let line = line.as_ref();
        buf.reserve(line.len() + 1);
        buf.put(line.as_bytes());
        buf.put_u8(b'\n');
        Ok(())
    }
}

impl Default for LinesCodec {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum LinesCodecError {
    MaxLineLengthExceeded,
    Io(io::Error),
}

impl fmt::Display for LinesCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinesCodecError::MaxLineLengthExceeded => write!(f, "max line length exceeded"),
            LinesCodecError::Io(e) => write!(f, "{}", e),
        }
    }
}

impl From<io::Error> for LinesCodecError {
    fn from(e: io::Error) -> LinesCodecError {
        LinesCodecError::Io(e)
    }
}

impl std::error::Error for LinesCodecError {}
