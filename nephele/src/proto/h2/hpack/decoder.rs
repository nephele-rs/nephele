use bytes::{Buf, Bytes, BytesMut};
use http::header;
use http::method::{self, Method};
use http::status::{self, StatusCode};
use std::cmp;
use std::collections::VecDeque;
use std::io::Cursor;
use std::str::Utf8Error;

use crate::proto::h2::frame;
use crate::proto::h2::hpack::{header::BytesStr, huffman, Header};

#[derive(Debug)]
pub struct Decoder {
    max_size_update: Option<usize>,
    last_max_update: usize,
    table: Table,
    buffer: BytesMut,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DecoderError {
    InvalidRepresentation,
    InvalidIntegerPrefix,
    InvalidTableIndex,
    InvalidHuffmanCode,
    InvalidUtf8,
    InvalidStatusCode,
    InvalidPseudoheader,
    InvalidMaxDynamicSize,
    IntegerOverflow,
    NeedMore(NeedMore),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum NeedMore {
    UnexpectedEndOfStream,
    IntegerUnderflow,
    StringUnderflow,
}

enum Representation {
    Indexed,
    LiteralWithIndexing,
    LiteralWithoutIndexing,
    LiteralNeverIndexed,
    SizeUpdate,
}

#[derive(Debug)]
struct Table {
    entries: VecDeque<Header>,
    size: usize,
    max_size: usize,
}

impl Decoder {
    pub fn new(size: usize) -> Decoder {
        Decoder {
            max_size_update: None,
            last_max_update: size,
            table: Table::new(size),
            buffer: BytesMut::with_capacity(4096),
        }
    }

    #[allow(dead_code)]
    pub fn queue_size_update(&mut self, size: usize) {
        let size = match self.max_size_update {
            Some(v) => cmp::max(v, size),
            None => size,
        };

        self.max_size_update = Some(size);
    }

    pub fn decode<F>(
        &mut self,
        src: &mut Cursor<&mut BytesMut>,
        mut f: F,
    ) -> Result<(), DecoderError>
    where
        F: FnMut(Header),
    {
        use self::Representation::*;

        let mut can_resize = true;

        if let Some(size) = self.max_size_update.take() {
            self.last_max_update = size;
        }

        let span = tracing::trace_span!("hpack::decode");
        let _e = span.enter();

        tracing::trace!("decode");

        while let Some(ty) = peek_u8(src) {
            match Representation::load(ty)? {
                Indexed => {
                    tracing::trace!(rem = src.remaining(), kind = %"Indexed");
                    can_resize = false;
                    let entry = self.decode_indexed(src)?;
                    consume(src);
                    f(entry);
                }
                LiteralWithIndexing => {
                    tracing::trace!(rem = src.remaining(), kind = %"LiteralWithIndexing");
                    can_resize = false;
                    let entry = self.decode_literal(src, true)?;

                    self.table.insert(entry.clone());
                    consume(src);

                    f(entry);
                }
                LiteralWithoutIndexing => {
                    tracing::trace!(rem = src.remaining(), kind = %"LiteralWithoutIndexing");
                    can_resize = false;
                    let entry = self.decode_literal(src, false)?;
                    consume(src);
                    f(entry);
                }
                LiteralNeverIndexed => {
                    tracing::trace!(rem = src.remaining(), kind = %"LiteralNeverIndexed");
                    can_resize = false;
                    let entry = self.decode_literal(src, false)?;
                    consume(src);

                    f(entry);
                }
                SizeUpdate => {
                    tracing::trace!(rem = src.remaining(), kind = %"SizeUpdate");
                    if !can_resize {
                        return Err(DecoderError::InvalidMaxDynamicSize);
                    }

                    self.process_size_update(src)?;
                    consume(src);
                }
            }
        }

        Ok(())
    }

    fn process_size_update(&mut self, buf: &mut Cursor<&mut BytesMut>) -> Result<(), DecoderError> {
        let new_size = decode_int(buf, 5)?;

        if new_size > self.last_max_update {
            return Err(DecoderError::InvalidMaxDynamicSize);
        }

        tracing::debug!(
            from = self.table.size(),
            to = new_size,
            "Decoder changed max table size"
        );

        self.table.set_max_size(new_size);

        Ok(())
    }

    fn decode_indexed(&self, buf: &mut Cursor<&mut BytesMut>) -> Result<Header, DecoderError> {
        let index = decode_int(buf, 7)?;
        self.table.get(index)
    }

    fn decode_literal(
        &mut self,
        buf: &mut Cursor<&mut BytesMut>,
        index: bool,
    ) -> Result<Header, DecoderError> {
        let prefix = if index { 6 } else { 4 };

        let table_idx = decode_int(buf, prefix)?;

        if table_idx == 0 {
            let name = self.decode_string(buf)?;
            let value = self.decode_string(buf)?;

            Header::new(name, value)
        } else {
            let e = self.table.get(table_idx)?;
            let value = self.decode_string(buf)?;

            e.name().into_entry(value)
        }
    }

    fn decode_string(&mut self, buf: &mut Cursor<&mut BytesMut>) -> Result<Bytes, DecoderError> {
        const HUFF_FLAG: u8 = 0b1000_0000;

        let huff = match peek_u8(buf) {
            Some(hdr) => (hdr & HUFF_FLAG) == HUFF_FLAG,
            None => return Err(DecoderError::NeedMore(NeedMore::UnexpectedEndOfStream)),
        };

        let len = decode_int(buf, 7)?;

        if len > buf.remaining() {
            tracing::trace!(len, remaining = buf.remaining(), "decode_string underflow",);
            return Err(DecoderError::NeedMore(NeedMore::StringUnderflow));
        }

        if huff {
            let ret = {
                let raw = &buf.chunk()[..len];
                huffman::decode(raw, &mut self.buffer).map(BytesMut::freeze)
            };

            buf.advance(len);
            return ret;
        }

        Ok(take(buf, len))
    }
}

impl Default for Decoder {
    fn default() -> Decoder {
        Decoder::new(4096)
    }
}

impl Representation {
    pub fn load(byte: u8) -> Result<Representation, DecoderError> {
        const INDEXED: u8 = 0b1000_0000;
        const LITERAL_WITH_INDEXING: u8 = 0b0100_0000;
        const LITERAL_WITHOUT_INDEXING: u8 = 0b1111_0000;
        const LITERAL_NEVER_INDEXED: u8 = 0b0001_0000;
        const SIZE_UPDATE_MASK: u8 = 0b1110_0000;
        const SIZE_UPDATE: u8 = 0b0010_0000;

        if byte & INDEXED == INDEXED {
            Ok(Representation::Indexed)
        } else if byte & LITERAL_WITH_INDEXING == LITERAL_WITH_INDEXING {
            Ok(Representation::LiteralWithIndexing)
        } else if byte & LITERAL_WITHOUT_INDEXING == 0 {
            Ok(Representation::LiteralWithoutIndexing)
        } else if byte & LITERAL_WITHOUT_INDEXING == LITERAL_NEVER_INDEXED {
            Ok(Representation::LiteralNeverIndexed)
        } else if byte & SIZE_UPDATE_MASK == SIZE_UPDATE {
            Ok(Representation::SizeUpdate)
        } else {
            Err(DecoderError::InvalidRepresentation)
        }
    }
}

fn decode_int<B: Buf>(buf: &mut B, prefix_size: u8) -> Result<usize, DecoderError> {
    const MAX_BYTES: usize = 5;
    const VARINT_MASK: u8 = 0b0111_1111;
    const VARINT_FLAG: u8 = 0b1000_0000;

    if prefix_size < 1 || prefix_size > 8 {
        return Err(DecoderError::InvalidIntegerPrefix);
    }

    if !buf.has_remaining() {
        return Err(DecoderError::NeedMore(NeedMore::IntegerUnderflow));
    }

    let mask = if prefix_size == 8 {
        0xFF
    } else {
        (1u8 << prefix_size).wrapping_sub(1)
    };

    let mut ret = (buf.get_u8() & mask) as usize;

    if ret < mask as usize {
        return Ok(ret);
    }

    let mut bytes = 1;

    let mut shift = 0;

    while buf.has_remaining() {
        let b = buf.get_u8();

        bytes += 1;
        ret += ((b & VARINT_MASK) as usize) << shift;
        shift += 7;

        if b & VARINT_FLAG == 0 {
            return Ok(ret);
        }

        if bytes == MAX_BYTES {
            return Err(DecoderError::IntegerOverflow);
        }
    }

    Err(DecoderError::NeedMore(NeedMore::IntegerUnderflow))
}

fn peek_u8<B: Buf>(buf: &mut B) -> Option<u8> {
    if buf.has_remaining() {
        Some(buf.chunk()[0])
    } else {
        None
    }
}

fn take(buf: &mut Cursor<&mut BytesMut>, n: usize) -> Bytes {
    let pos = buf.position() as usize;
    let mut head = buf.get_mut().split_to(pos + n);
    buf.set_position(0);
    head.advance(pos);
    head.freeze()
}

fn consume(buf: &mut Cursor<&mut BytesMut>) {
    take(buf, 0);
}

impl Table {
    fn new(max_size: usize) -> Table {
        Table {
            entries: VecDeque::new(),
            size: 0,
            max_size,
        }
    }

    fn size(&self) -> usize {
        self.size
    }

    pub fn get(&self, index: usize) -> Result<Header, DecoderError> {
        if index == 0 {
            return Err(DecoderError::InvalidTableIndex);
        }

        if index <= 61 {
            return Ok(get_static(index));
        }

        match self.entries.get(index - 62) {
            Some(e) => Ok(e.clone()),
            None => Err(DecoderError::InvalidTableIndex),
        }
    }

    fn insert(&mut self, entry: Header) {
        let len = entry.len();

        self.reserve(len);

        if self.size + len <= self.max_size {
            self.size += len;

            self.entries.push_front(entry);
        }
    }

    fn set_max_size(&mut self, size: usize) {
        self.max_size = size;
        self.consolidate();
    }

    fn reserve(&mut self, size: usize) {
        while self.size + size > self.max_size {
            match self.entries.pop_back() {
                Some(last) => {
                    self.size -= last.len();
                }
                None => return,
            }
        }
    }

    fn consolidate(&mut self) {
        while self.size > self.max_size {
            {
                let last = match self.entries.back() {
                    Some(x) => x,
                    None => {
                        panic!("Size of table != 0, but no headers left!");
                    }
                };

                self.size -= last.len();
            }

            self.entries.pop_back();
        }
    }
}

impl From<Utf8Error> for DecoderError {
    fn from(_: Utf8Error) -> DecoderError {
        DecoderError::InvalidUtf8
    }
}

impl From<header::InvalidHeaderValue> for DecoderError {
    fn from(_: header::InvalidHeaderValue) -> DecoderError {
        DecoderError::InvalidUtf8
    }
}

impl From<header::InvalidHeaderName> for DecoderError {
    fn from(_: header::InvalidHeaderName) -> DecoderError {
        DecoderError::InvalidUtf8
    }
}

impl From<method::InvalidMethod> for DecoderError {
    fn from(_: method::InvalidMethod) -> DecoderError {
        DecoderError::InvalidUtf8
    }
}

impl From<status::InvalidStatusCode> for DecoderError {
    fn from(_: status::InvalidStatusCode) -> DecoderError {
        DecoderError::InvalidUtf8
    }
}

impl From<DecoderError> for frame::Error {
    fn from(src: DecoderError) -> Self {
        frame::Error::Hpack(src)
    }
}

pub fn get_static(idx: usize) -> Header {
    use http::header::HeaderValue;

    match idx {
        1 => Header::Authority(from_static("")),
        2 => Header::Method(Method::GET),
        3 => Header::Method(Method::POST),
        4 => Header::Path(from_static("/")),
        5 => Header::Path(from_static("/index.html")),
        6 => Header::Scheme(from_static("http")),
        7 => Header::Scheme(from_static("https")),
        8 => Header::Status(StatusCode::OK),
        9 => Header::Status(StatusCode::NO_CONTENT),
        10 => Header::Status(StatusCode::PARTIAL_CONTENT),
        11 => Header::Status(StatusCode::NOT_MODIFIED),
        12 => Header::Status(StatusCode::BAD_REQUEST),
        13 => Header::Status(StatusCode::NOT_FOUND),
        14 => Header::Status(StatusCode::INTERNAL_SERVER_ERROR),
        15 => Header::Field {
            name: header::ACCEPT_CHARSET,
            value: HeaderValue::from_static(""),
        },
        16 => Header::Field {
            name: header::ACCEPT_ENCODING,
            value: HeaderValue::from_static("gzip, deflate"),
        },
        17 => Header::Field {
            name: header::ACCEPT_LANGUAGE,
            value: HeaderValue::from_static(""),
        },
        18 => Header::Field {
            name: header::ACCEPT_RANGES,
            value: HeaderValue::from_static(""),
        },
        19 => Header::Field {
            name: header::ACCEPT,
            value: HeaderValue::from_static(""),
        },
        20 => Header::Field {
            name: header::ACCESS_CONTROL_ALLOW_ORIGIN,
            value: HeaderValue::from_static(""),
        },
        21 => Header::Field {
            name: header::AGE,
            value: HeaderValue::from_static(""),
        },
        22 => Header::Field {
            name: header::ALLOW,
            value: HeaderValue::from_static(""),
        },
        23 => Header::Field {
            name: header::AUTHORIZATION,
            value: HeaderValue::from_static(""),
        },
        24 => Header::Field {
            name: header::CACHE_CONTROL,
            value: HeaderValue::from_static(""),
        },
        25 => Header::Field {
            name: header::CONTENT_DISPOSITION,
            value: HeaderValue::from_static(""),
        },
        26 => Header::Field {
            name: header::CONTENT_ENCODING,
            value: HeaderValue::from_static(""),
        },
        27 => Header::Field {
            name: header::CONTENT_LANGUAGE,
            value: HeaderValue::from_static(""),
        },
        28 => Header::Field {
            name: header::CONTENT_LENGTH,
            value: HeaderValue::from_static(""),
        },
        29 => Header::Field {
            name: header::CONTENT_LOCATION,
            value: HeaderValue::from_static(""),
        },
        30 => Header::Field {
            name: header::CONTENT_RANGE,
            value: HeaderValue::from_static(""),
        },
        31 => Header::Field {
            name: header::CONTENT_TYPE,
            value: HeaderValue::from_static(""),
        },
        32 => Header::Field {
            name: header::COOKIE,
            value: HeaderValue::from_static(""),
        },
        33 => Header::Field {
            name: header::DATE,
            value: HeaderValue::from_static(""),
        },
        34 => Header::Field {
            name: header::ETAG,
            value: HeaderValue::from_static(""),
        },
        35 => Header::Field {
            name: header::EXPECT,
            value: HeaderValue::from_static(""),
        },
        36 => Header::Field {
            name: header::EXPIRES,
            value: HeaderValue::from_static(""),
        },
        37 => Header::Field {
            name: header::FROM,
            value: HeaderValue::from_static(""),
        },
        38 => Header::Field {
            name: header::HOST,
            value: HeaderValue::from_static(""),
        },
        39 => Header::Field {
            name: header::IF_MATCH,
            value: HeaderValue::from_static(""),
        },
        40 => Header::Field {
            name: header::IF_MODIFIED_SINCE,
            value: HeaderValue::from_static(""),
        },
        41 => Header::Field {
            name: header::IF_NONE_MATCH,
            value: HeaderValue::from_static(""),
        },
        42 => Header::Field {
            name: header::IF_RANGE,
            value: HeaderValue::from_static(""),
        },
        43 => Header::Field {
            name: header::IF_UNMODIFIED_SINCE,
            value: HeaderValue::from_static(""),
        },
        44 => Header::Field {
            name: header::LAST_MODIFIED,
            value: HeaderValue::from_static(""),
        },
        45 => Header::Field {
            name: header::LINK,
            value: HeaderValue::from_static(""),
        },
        46 => Header::Field {
            name: header::LOCATION,
            value: HeaderValue::from_static(""),
        },
        47 => Header::Field {
            name: header::MAX_FORWARDS,
            value: HeaderValue::from_static(""),
        },
        48 => Header::Field {
            name: header::PROXY_AUTHENTICATE,
            value: HeaderValue::from_static(""),
        },
        49 => Header::Field {
            name: header::PROXY_AUTHORIZATION,
            value: HeaderValue::from_static(""),
        },
        50 => Header::Field {
            name: header::RANGE,
            value: HeaderValue::from_static(""),
        },
        51 => Header::Field {
            name: header::REFERER,
            value: HeaderValue::from_static(""),
        },
        52 => Header::Field {
            name: header::REFRESH,
            value: HeaderValue::from_static(""),
        },
        53 => Header::Field {
            name: header::RETRY_AFTER,
            value: HeaderValue::from_static(""),
        },
        54 => Header::Field {
            name: header::SERVER,
            value: HeaderValue::from_static(""),
        },
        55 => Header::Field {
            name: header::SET_COOKIE,
            value: HeaderValue::from_static(""),
        },
        56 => Header::Field {
            name: header::STRICT_TRANSPORT_SECURITY,
            value: HeaderValue::from_static(""),
        },
        57 => Header::Field {
            name: header::TRANSFER_ENCODING,
            value: HeaderValue::from_static(""),
        },
        58 => Header::Field {
            name: header::USER_AGENT,
            value: HeaderValue::from_static(""),
        },
        59 => Header::Field {
            name: header::VARY,
            value: HeaderValue::from_static(""),
        },
        60 => Header::Field {
            name: header::VIA,
            value: HeaderValue::from_static(""),
        },
        61 => Header::Field {
            name: header::WWW_AUTHENTICATE,
            value: HeaderValue::from_static(""),
        },
        _ => unreachable!(),
    }
}

fn from_static(s: &'static str) -> BytesStr {
    unsafe { BytesStr::from_utf8_unchecked(Bytes::from_static(s.as_bytes())) }
}
