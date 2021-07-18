use cynthia::future::swap::{AsyncRead, BufferReader, Take};
use cynthia::platform::dup::{Arc, Mutex};
use cynthia::runtime::task::{Context, Poll};
use std::{fmt::Debug, io, pin::Pin};

use crate::proto::h1::chunked::ChunkedDecoder;

pub enum BodyReader<IO: AsyncRead + Unpin> {
    Chunked(Arc<Mutex<ChunkedDecoder<BufferReader<IO>>>>),
    Fixed(Arc<Mutex<Take<BufferReader<IO>>>>),
    None,
}

impl<IO: AsyncRead + Unpin> Debug for BodyReader<IO> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BodyReader::Chunked(_) => f.write_str("BodyReader::Chunked"),
            BodyReader::Fixed(_) => f.write_str("BodyReader::Fixed"),
            BodyReader::None => f.write_str("BodyReader::None"),
        }
    }
}

impl<IO: AsyncRead + Unpin> AsyncRead for BodyReader<IO> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        match &*self {
            BodyReader::Chunked(r) => Pin::new(&mut *r.lock()).poll_read(cx, buf),
            BodyReader::Fixed(r) => Pin::new(&mut *r.lock()).poll_read(cx, buf),
            BodyReader::None => Poll::Ready(Ok(0)),
        }
    }
}
