use bytes::{Bytes, BytesMut};
use http::header::{self, HeaderName, HeaderValue};
use http::{uri, HeaderMap, Method, Request, StatusCode, Uri};
use std::fmt;
use std::io::Cursor;

use crate::proto::h2::frame::{util, StreamDependency, StreamId};
use crate::proto::h2::frame::{Error, Frame, Head, Kind};
use crate::proto::h2::hpack::{self, BytesStr};

type EncodeBuf<'a> = bytes::buf::Limit<&'a mut BytesMut>;

const MAX_HEADER_LENGTH: usize = 1024 * 16 - 100;

#[derive(Eq, PartialEq)]
pub struct Headers {
    stream_id: StreamId,

    stream_dep: Option<StreamDependency>,

    header_block: HeaderBlock,

    flags: HeadersFlag,
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub struct HeadersFlag(u8);

#[derive(Eq, PartialEq)]
pub struct PushPromise {
    stream_id: StreamId,

    promised_id: StreamId,

    header_block: HeaderBlock,

    flags: PushPromiseFlag,
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub struct PushPromiseFlag(u8);

#[derive(Debug)]
pub struct Continuation {
    stream_id: StreamId,

    header_block: EncodingHeaderBlock,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct Pseudo {
    pub method: Option<Method>,
    pub scheme: Option<BytesStr>,
    pub authority: Option<BytesStr>,
    pub path: Option<BytesStr>,

    pub status: Option<StatusCode>,
}

#[derive(Debug)]
pub struct Iter {
    pseudo: Option<Pseudo>,

    fields: header::IntoIter<HeaderValue>,
}

#[derive(Debug, PartialEq, Eq)]
struct HeaderBlock {
    fields: HeaderMap,

    is_over_size: bool,

    pseudo: Pseudo,
}

#[derive(Debug)]
struct EncodingHeaderBlock {
    hpack: Option<hpack::EncodeState>,

    headers: Iter,
}

const END_STREAM: u8 = 0x1;
const END_HEADERS: u8 = 0x4;
const PADDED: u8 = 0x8;
const PRIORITY: u8 = 0x20;
const ALL: u8 = END_STREAM | END_HEADERS | PADDED | PRIORITY;

impl Headers {
    pub fn new(stream_id: StreamId, pseudo: Pseudo, fields: HeaderMap) -> Self {
        Headers {
            stream_id,
            stream_dep: None,
            header_block: HeaderBlock {
                fields,
                is_over_size: false,
                pseudo,
            },
            flags: HeadersFlag::default(),
        }
    }

    pub fn trailers(stream_id: StreamId, fields: HeaderMap) -> Self {
        let mut flags = HeadersFlag::default();
        flags.set_end_stream();

        Headers {
            stream_id,
            stream_dep: None,
            header_block: HeaderBlock {
                fields,
                is_over_size: false,
                pseudo: Pseudo::default(),
            },
            flags,
        }
    }

    pub fn load(head: Head, mut src: BytesMut) -> Result<(Self, BytesMut), Error> {
        let flags = HeadersFlag(head.flag());
        let mut pad = 0;

        tracing::trace!("loading headers; flags={:?}", flags);

        if flags.is_padded() {
            if src.is_empty() {
                return Err(Error::MalformedMessage);
            }
            pad = src[0] as usize;

            let _ = src.split_to(1);
        }

        let stream_dep = if flags.is_priority() {
            if src.len() < 5 {
                return Err(Error::MalformedMessage);
            }
            let stream_dep = StreamDependency::load(&src[..5])?;

            if stream_dep.dependency_id() == head.stream_id() {
                return Err(Error::InvalidDependencyId);
            }

            let _ = src.split_to(5);

            Some(stream_dep)
        } else {
            None
        };

        if pad > 0 {
            if pad > src.len() {
                return Err(Error::TooMuchPadding);
            }

            let len = src.len() - pad;
            src.truncate(len);
        }

        let headers = Headers {
            stream_id: head.stream_id(),
            stream_dep,
            header_block: HeaderBlock {
                fields: HeaderMap::new(),
                is_over_size: false,
                pseudo: Pseudo::default(),
            },
            flags,
        };

        Ok((headers, src))
    }

    pub fn load_hpack(
        &mut self,
        src: &mut BytesMut,
        max_header_list_size: usize,
        decoder: &mut hpack::Decoder,
    ) -> Result<(), Error> {
        self.header_block.load(src, max_header_list_size, decoder)
    }

    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    pub fn is_end_headers(&self) -> bool {
        self.flags.is_end_headers()
    }

    pub fn set_end_headers(&mut self) {
        self.flags.set_end_headers();
    }

    pub fn is_end_stream(&self) -> bool {
        self.flags.is_end_stream()
    }

    pub fn set_end_stream(&mut self) {
        self.flags.set_end_stream()
    }

    pub fn is_over_size(&self) -> bool {
        self.header_block.is_over_size
    }

    pub(crate) fn has_too_big_field(&self) -> bool {
        self.header_block.has_too_big_field()
    }

    pub fn into_parts(self) -> (Pseudo, HeaderMap) {
        (self.header_block.pseudo, self.header_block.fields)
    }

    #[cfg(feature = "unstable")]
    pub fn pseudo_mut(&mut self) -> &mut Pseudo {
        &mut self.header_block.pseudo
    }

    pub fn fields(&self) -> &HeaderMap {
        &self.header_block.fields
    }

    pub fn into_fields(self) -> HeaderMap {
        self.header_block.fields
    }

    pub fn encode(
        self,
        encoder: &mut hpack::Encoder,
        dst: &mut EncodeBuf<'_>,
    ) -> Option<Continuation> {
        debug_assert!(self.flags.is_end_headers());

        let head = self.head();

        self.header_block
            .into_encoding()
            .encode(&head, encoder, dst, |_| {})
    }

    fn head(&self) -> Head {
        Head::new(Kind::Headers, self.flags.into(), self.stream_id)
    }
}

impl<T> From<Headers> for Frame<T> {
    fn from(src: Headers) -> Self {
        Frame::Headers(src)
    }
}

impl fmt::Debug for Headers {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut builder = f.debug_struct("Headers");
        builder
            .field("stream_id", &self.stream_id)
            .field("flags", &self.flags);

        if let Some(ref dep) = self.stream_dep {
            builder.field("stream_dep", dep);
        }

        builder.finish()
    }
}

pub fn parse_u64(src: &[u8]) -> Result<u64, ()> {
    if src.len() > 19 {
        return Err(());
    }

    let mut ret = 0;

    for &d in src {
        if d < b'0' || d > b'9' {
            return Err(());
        }

        ret *= 10;
        ret += (d - b'0') as u64;
    }

    Ok(ret)
}

#[derive(Debug)]
pub enum PushPromiseHeaderError {
    InvalidContentLength(Result<u64, ()>),
    NotSafeAndCacheable,
}

impl PushPromise {
    pub fn new(
        stream_id: StreamId,
        promised_id: StreamId,
        pseudo: Pseudo,
        fields: HeaderMap,
    ) -> Self {
        PushPromise {
            flags: PushPromiseFlag::default(),
            header_block: HeaderBlock {
                fields,
                is_over_size: false,
                pseudo,
            },
            promised_id,
            stream_id,
        }
    }

    pub fn validate_request(req: &Request<()>) -> Result<(), PushPromiseHeaderError> {
        use PushPromiseHeaderError::*;
        if let Some(content_length) = req.headers().get(header::CONTENT_LENGTH) {
            let parsed_length = parse_u64(content_length.as_bytes());
            if parsed_length != Ok(0) {
                return Err(InvalidContentLength(parsed_length));
            }
        }
        if !Self::safe_and_cacheable(req.method()) {
            return Err(NotSafeAndCacheable);
        }

        Ok(())
    }

    fn safe_and_cacheable(method: &Method) -> bool {
        return method == Method::GET || method == Method::HEAD;
    }

    pub fn fields(&self) -> &HeaderMap {
        &self.header_block.fields
    }

    #[cfg(feature = "unstable")]
    pub fn into_fields(self) -> HeaderMap {
        self.header_block.fields
    }

    pub fn load(head: Head, mut src: BytesMut) -> Result<(Self, BytesMut), Error> {
        let flags = PushPromiseFlag(head.flag());
        let mut pad = 0;

        if flags.is_padded() {
            if src.is_empty() {
                return Err(Error::MalformedMessage);
            }

            pad = src[0] as usize;

            let _ = src.split_to(1);
        }

        if src.len() < 5 {
            return Err(Error::MalformedMessage);
        }

        let (promised_id, _) = StreamId::parse(&src[..4]);
        let _ = src.split_to(4);

        if pad > 0 {
            if pad > src.len() {
                return Err(Error::TooMuchPadding);
            }

            let len = src.len() - pad;
            src.truncate(len);
        }

        let frame = PushPromise {
            flags,
            header_block: HeaderBlock {
                fields: HeaderMap::new(),
                is_over_size: false,
                pseudo: Pseudo::default(),
            },
            promised_id,
            stream_id: head.stream_id(),
        };
        Ok((frame, src))
    }

    pub fn load_hpack(
        &mut self,
        src: &mut BytesMut,
        max_header_list_size: usize,
        decoder: &mut hpack::Decoder,
    ) -> Result<(), Error> {
        self.header_block.load(src, max_header_list_size, decoder)
    }

    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    pub fn promised_id(&self) -> StreamId {
        self.promised_id
    }

    pub fn is_end_headers(&self) -> bool {
        self.flags.is_end_headers()
    }

    pub fn set_end_headers(&mut self) {
        self.flags.set_end_headers();
    }

    pub fn is_over_size(&self) -> bool {
        self.header_block.is_over_size
    }

    pub fn encode(
        self,
        encoder: &mut hpack::Encoder,
        dst: &mut EncodeBuf<'_>,
    ) -> Option<Continuation> {
        use bytes::BufMut;

        debug_assert!(self.flags.is_end_headers());

        let head = self.head();
        let promised_id = self.promised_id;

        self.header_block
            .into_encoding()
            .encode(&head, encoder, dst, |dst| {
                dst.put_u32(promised_id.into());
            })
    }

    fn head(&self) -> Head {
        Head::new(Kind::PushPromise, self.flags.into(), self.stream_id)
    }

    pub fn into_parts(self) -> (Pseudo, HeaderMap) {
        (self.header_block.pseudo, self.header_block.fields)
    }
}

impl<T> From<PushPromise> for Frame<T> {
    fn from(src: PushPromise) -> Self {
        Frame::PushPromise(src)
    }
}

impl fmt::Debug for PushPromise {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("PushPromise")
            .field("stream_id", &self.stream_id)
            .field("promised_id", &self.promised_id)
            .field("flags", &self.flags)
            .finish()
    }
}

impl Continuation {
    fn head(&self) -> Head {
        Head::new(Kind::Continuation, END_HEADERS, self.stream_id)
    }

    pub fn encode(
        self,
        encoder: &mut hpack::Encoder,
        dst: &mut EncodeBuf<'_>,
    ) -> Option<Continuation> {
        let head = self.head();

        self.header_block.encode(&head, encoder, dst, |_| {})
    }
}

impl Pseudo {
    pub fn request(method: Method, uri: Uri) -> Self {
        let parts = uri::Parts::from(uri);

        let mut path = parts
            .path_and_query
            .map(|v| Bytes::copy_from_slice(v.as_str().as_bytes()))
            .unwrap_or_else(Bytes::new);

        if path.is_empty() && method != Method::OPTIONS {
            path = Bytes::from_static(b"/");
        }

        let mut pseudo = Pseudo {
            method: Some(method),
            scheme: None,
            authority: None,
            path: Some(unsafe { BytesStr::from_utf8_unchecked(path) }),
            status: None,
        };

        if let Some(scheme) = parts.scheme {
            pseudo.set_scheme(scheme);
        }

        if let Some(authority) = parts.authority {
            pseudo.set_authority(unsafe {
                BytesStr::from_utf8_unchecked(Bytes::copy_from_slice(authority.as_str().as_bytes()))
            });
        }

        pseudo
    }

    pub fn response(status: StatusCode) -> Self {
        Pseudo {
            method: None,
            scheme: None,
            authority: None,
            path: None,
            status: Some(status),
        }
    }

    pub fn set_scheme(&mut self, scheme: uri::Scheme) {
        let bytes = match scheme.as_str() {
            "http" => Bytes::from_static(b"http"),
            "https" => Bytes::from_static(b"https"),
            s => Bytes::copy_from_slice(s.as_bytes()),
        };
        self.scheme = Some(unsafe { BytesStr::from_utf8_unchecked(bytes) });
    }

    pub fn set_authority(&mut self, authority: BytesStr) {
        self.authority = Some(authority);
    }
}

impl EncodingHeaderBlock {
    fn encode<F>(
        mut self,
        head: &Head,
        encoder: &mut hpack::Encoder,
        dst: &mut EncodeBuf<'_>,
        f: F,
    ) -> Option<Continuation>
    where
        F: FnOnce(&mut EncodeBuf<'_>),
    {
        let head_pos = dst.get_ref().len();

        head.encode(0, dst);

        let payload_pos = dst.get_ref().len();

        f(dst);

        let continuation = match encoder.encode(self.hpack, &mut self.headers, dst) {
            hpack::Encode::Full => None,
            hpack::Encode::Partial(state) => Some(Continuation {
                stream_id: head.stream_id(),
                header_block: EncodingHeaderBlock {
                    hpack: Some(state),
                    headers: self.headers,
                },
            }),
        };

        let payload_len = (dst.get_ref().len() - payload_pos) as u64;

        let payload_len_be = payload_len.to_be_bytes();
        assert!(payload_len_be[0..5].iter().all(|b| *b == 0));
        (dst.get_mut()[head_pos..head_pos + 3]).copy_from_slice(&payload_len_be[5..]);

        if continuation.is_some() {
            debug_assert!(dst.get_ref()[head_pos + 4] & END_HEADERS == END_HEADERS);

            dst.get_mut()[head_pos + 4] -= END_HEADERS;
        }

        continuation
    }
}

impl Iterator for Iter {
    type Item = hpack::Header<Option<HeaderName>>;

    fn next(&mut self) -> Option<Self::Item> {
        use crate::proto::h2::hpack::Header::*;

        if let Some(ref mut pseudo) = self.pseudo {
            if let Some(method) = pseudo.method.take() {
                return Some(Method(method));
            }

            if let Some(scheme) = pseudo.scheme.take() {
                return Some(Scheme(scheme));
            }

            if let Some(authority) = pseudo.authority.take() {
                return Some(Authority(authority));
            }

            if let Some(path) = pseudo.path.take() {
                return Some(Path(path));
            }

            if let Some(status) = pseudo.status.take() {
                return Some(Status(status));
            }
        }

        self.pseudo = None;

        self.fields
            .next()
            .map(|(name, value)| Field { name, value })
    }
}

impl HeadersFlag {
    pub fn empty() -> HeadersFlag {
        HeadersFlag(0)
    }

    pub fn load(bits: u8) -> HeadersFlag {
        HeadersFlag(bits & ALL)
    }

    pub fn is_end_stream(&self) -> bool {
        self.0 & END_STREAM == END_STREAM
    }

    pub fn set_end_stream(&mut self) {
        self.0 |= END_STREAM;
    }

    pub fn is_end_headers(&self) -> bool {
        self.0 & END_HEADERS == END_HEADERS
    }

    pub fn set_end_headers(&mut self) {
        self.0 |= END_HEADERS;
    }

    pub fn is_padded(&self) -> bool {
        self.0 & PADDED == PADDED
    }

    pub fn is_priority(&self) -> bool {
        self.0 & PRIORITY == PRIORITY
    }
}

impl Default for HeadersFlag {
    fn default() -> Self {
        HeadersFlag(END_HEADERS)
    }
}

impl From<HeadersFlag> for u8 {
    fn from(src: HeadersFlag) -> u8 {
        src.0
    }
}

impl fmt::Debug for HeadersFlag {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        util::debug_flags(fmt, self.0)
            .flag_if(self.is_end_headers(), "END_HEADERS")
            .flag_if(self.is_end_stream(), "END_STREAM")
            .flag_if(self.is_padded(), "PADDED")
            .flag_if(self.is_priority(), "PRIORITY")
            .finish()
    }
}

impl PushPromiseFlag {
    pub fn empty() -> PushPromiseFlag {
        PushPromiseFlag(0)
    }

    pub fn load(bits: u8) -> PushPromiseFlag {
        PushPromiseFlag(bits & ALL)
    }

    pub fn is_end_headers(&self) -> bool {
        self.0 & END_HEADERS == END_HEADERS
    }

    pub fn set_end_headers(&mut self) {
        self.0 |= END_HEADERS;
    }

    pub fn is_padded(&self) -> bool {
        self.0 & PADDED == PADDED
    }
}

impl Default for PushPromiseFlag {
    fn default() -> Self {
        PushPromiseFlag(END_HEADERS)
    }
}

impl From<PushPromiseFlag> for u8 {
    fn from(src: PushPromiseFlag) -> u8 {
        src.0
    }
}

impl fmt::Debug for PushPromiseFlag {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        util::debug_flags(fmt, self.0)
            .flag_if(self.is_end_headers(), "END_HEADERS")
            .flag_if(self.is_padded(), "PADDED")
            .finish()
    }
}

impl HeaderBlock {
    fn load(
        &mut self,
        src: &mut BytesMut,
        max_header_list_size: usize,
        decoder: &mut hpack::Decoder,
    ) -> Result<(), Error> {
        let mut reg = !self.fields.is_empty();
        let mut malformed = false;
        let mut headers_size = self.calculate_header_list_size();

        macro_rules! set_pseudo {
            ($field:ident, $val:expr) => {{
                if reg {
                    tracing::trace!("load_hpack; header malformed -- pseudo not at head of block");
                    malformed = true;
                } else if self.pseudo.$field.is_some() {
                    tracing::trace!("load_hpack; header malformed -- repeated pseudo");
                    malformed = true;
                } else {
                    let __val = $val;
                    headers_size +=
                        decoded_header_size(stringify!($ident).len() + 1, __val.as_str().len());
                    if headers_size < max_header_list_size {
                        self.pseudo.$field = Some(__val);
                    } else if !self.is_over_size {
                        tracing::trace!("load_hpack; header list size over max");
                        self.is_over_size = true;
                    }
                }
            }};
        }

        let mut cursor = Cursor::new(src);

        let res = decoder.decode(&mut cursor, |header| {
            use crate::proto::h2::hpack::Header::*;

            match header {
                Field { name, value } => {
                    if name == header::CONNECTION
                        || name == header::TRANSFER_ENCODING
                        || name == header::UPGRADE
                        || name == "keep-alive"
                        || name == "proxy-connection"
                    {
                        tracing::trace!("load_hpack; connection level header");
                        malformed = true;
                    } else if name == header::TE && value != "trailers" {
                        tracing::trace!(
                            "load_hpack; TE header not set to trailers; val={:?}",
                            value
                        );
                        malformed = true;
                    } else {
                        reg = true;

                        headers_size += decoded_header_size(name.as_str().len(), value.len());
                        if headers_size < max_header_list_size {
                            self.fields.append(name, value);
                        } else if !self.is_over_size {
                            tracing::trace!("load_hpack; header list size over max");
                            self.is_over_size = true;
                        }
                    }
                }
                Authority(v) => set_pseudo!(authority, v),
                Method(v) => set_pseudo!(method, v),
                Scheme(v) => set_pseudo!(scheme, v),
                Path(v) => set_pseudo!(path, v),
                Status(v) => set_pseudo!(status, v),
            }
        });

        if let Err(e) = res {
            tracing::trace!("hpack decoding error; err={:?}", e);
            return Err(e.into());
        }

        if malformed {
            tracing::trace!("malformed message");
            return Err(Error::MalformedMessage);
        }

        Ok(())
    }

    fn into_encoding(self) -> EncodingHeaderBlock {
        EncodingHeaderBlock {
            hpack: None,
            headers: Iter {
                pseudo: Some(self.pseudo),
                fields: self.fields.into_iter(),
            },
        }
    }

    fn calculate_header_list_size(&self) -> usize {
        macro_rules! pseudo_size {
            ($name:ident) => {{
                self.pseudo
                    .$name
                    .as_ref()
                    .map(|m| decoded_header_size(stringify!($name).len() + 1, m.as_str().len()))
                    .unwrap_or(0)
            }};
        }

        pseudo_size!(method)
            + pseudo_size!(scheme)
            + pseudo_size!(status)
            + pseudo_size!(authority)
            + pseudo_size!(path)
            + self
                .fields
                .iter()
                .map(|(name, value)| decoded_header_size(name.as_str().len(), value.len()))
                .sum::<usize>()
    }

    pub(crate) fn has_too_big_field(&self) -> bool {
        macro_rules! pseudo_size {
            ($name:ident) => {{
                self.pseudo
                    .$name
                    .as_ref()
                    .map(|m| decoded_header_size(stringify!($name).len() + 1, m.as_str().len()))
                    .unwrap_or(0)
            }};
        }

        if pseudo_size!(method) > MAX_HEADER_LENGTH {
            return true;
        }

        if pseudo_size!(scheme) > MAX_HEADER_LENGTH {
            return true;
        }

        if pseudo_size!(authority) > MAX_HEADER_LENGTH {
            return true;
        }

        if pseudo_size!(path) > MAX_HEADER_LENGTH {
            return true;
        }

        for (name, value) in &self.fields {
            if decoded_header_size(name.as_str().len(), value.len()) > MAX_HEADER_LENGTH {
                return true;
            }
        }

        false
    }
}

fn decoded_header_size(name: usize, value: usize) -> usize {
    name + value + 32
}
