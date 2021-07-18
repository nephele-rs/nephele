use bytes::BytesMut;
use cynthia::future::swap::{AsyncRead, AsyncWrite};
use futures_core::stream::Stream;
use futures_sink::Sink;
use pin_project_lite::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{fmt, io};

use crate::common::codec::decoder::Decoder;
use crate::common::codec::encoder::Encoder;
use crate::common::codec::framed_impl::{FramedImpl, RWFrames, ReadFrame, WriteFrame};

pin_project! {
    pub struct Framed<T, U> {
        #[pin]
        inner: FramedImpl<T, U, RWFrames>
    }
}

impl<T, U> Framed<T, U>
where
    T: AsyncRead + AsyncWrite,
{
    pub fn new(inner: T, codec: U) -> Framed<T, U> {
        Framed {
            inner: FramedImpl {
                inner,
                codec,
                state: Default::default(),
            },
        }
    }

    pub fn with_capacity(inner: T, codec: U, capacity: usize) -> Framed<T, U> {
        Framed {
            inner: FramedImpl {
                inner,
                codec,
                state: RWFrames {
                    read: ReadFrame {
                        eof: false,
                        is_readable: false,
                        buffer: BytesMut::with_capacity(capacity),
                    },
                    write: WriteFrame::default(),
                },
            },
        }
    }
}

impl<T, U> Framed<T, U> {
    pub fn from_parts(parts: FramedParts<T, U>) -> Framed<T, U> {
        Framed {
            inner: FramedImpl {
                inner: parts.io,
                codec: parts.codec,
                state: RWFrames {
                    read: parts.read_buf.into(),
                    write: parts.write_buf.into(),
                },
            },
        }
    }

    pub fn get_ref(&self) -> &T {
        &self.inner.inner
    }

    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner.inner
    }

    pub fn get_pin_mut(self: Pin<&mut Self>) -> Pin<&mut T> {
        self.project().inner.project().inner
    }

    pub fn codec(&self) -> &U {
        &self.inner.codec
    }

    pub fn codec_mut(&mut self) -> &mut U {
        &mut self.inner.codec
    }

    pub fn read_buffer(&self) -> &BytesMut {
        &self.inner.state.read.buffer
    }

    pub fn read_buffer_mut(&mut self) -> &mut BytesMut {
        &mut self.inner.state.read.buffer
    }

    pub fn write_buffer(&self) -> &BytesMut {
        &self.inner.state.write.buffer
    }

    pub fn write_buffer_mut(&mut self) -> &mut BytesMut {
        &mut self.inner.state.write.buffer
    }

    pub fn into_inner(self) -> T {
        self.inner.inner
    }

    pub fn into_parts(self) -> FramedParts<T, U> {
        FramedParts {
            io: self.inner.inner,
            codec: self.inner.codec,
            read_buf: self.inner.state.read.buffer,
            write_buf: self.inner.state.write.buffer,
            _priv: (),
        }
    }
}

impl<T, U> Stream for Framed<T, U>
where
    T: AsyncRead,
    U: Decoder,
{
    type Item = Result<U::Item, U::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().inner.poll_next(cx)
    }
}

impl<T, I, U> Sink<I> for Framed<T, U>
where
    T: AsyncWrite,
    U: Encoder<I>,
    U::Error: From<io::Error>,
{
    type Error = U::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().inner.poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: I) -> Result<(), Self::Error> {
        self.project().inner.start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().inner.poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().inner.poll_close(cx)
    }
}

impl<T, U> fmt::Debug for Framed<T, U>
where
    T: fmt::Debug,
    U: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Framed")
            .field("io", self.get_ref())
            .field("codec", self.codec())
            .finish()
    }
}

#[derive(Debug)]
#[allow(clippy::manual_non_exhaustive)]
pub struct FramedParts<T, U> {
    pub io: T,

    pub codec: U,

    pub read_buf: BytesMut,

    pub write_buf: BytesMut,

    _priv: (),
}

impl<T, U> FramedParts<T, U> {
    pub fn new<I>(io: T, codec: U) -> FramedParts<T, U>
    where
        U: Encoder<I>,
    {
        FramedParts {
            io,
            codec,
            read_buf: BytesMut::new(),
            write_buf: BytesMut::new(),
            _priv: (),
        }
    }
}
