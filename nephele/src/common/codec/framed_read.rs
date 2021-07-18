use bytes::BytesMut;
use cynthia::future::swap::AsyncRead;
use futures_core::stream::Stream;
use futures_sink::Sink;
use pin_project_lite::pin_project;
use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::common::codec::framed_impl::{FramedImpl, ReadFrame};
use crate::common::codec::Decoder;

pin_project! {
    pub struct FramedRead<T, D> {
        #[pin]
        inner: FramedImpl<T, D, ReadFrame>,
    }
}

impl<T, D> FramedRead<T, D>
where
    T: AsyncRead,
    D: Decoder,
{
    pub fn new(inner: T, decoder: D) -> FramedRead<T, D> {
        FramedRead {
            inner: FramedImpl {
                inner,
                codec: decoder,
                state: Default::default(),
            },
        }
    }

    pub fn with_capacity(inner: T, decoder: D, capacity: usize) -> FramedRead<T, D> {
        FramedRead {
            inner: FramedImpl {
                inner,
                codec: decoder,
                state: ReadFrame {
                    eof: false,
                    is_readable: false,
                    buffer: BytesMut::with_capacity(capacity),
                },
            },
        }
    }
}

impl<T, D> FramedRead<T, D> {
    pub fn get_ref(&self) -> &T {
        &self.inner.inner
    }

    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner.inner
    }

    pub fn get_pin_mut(self: Pin<&mut Self>) -> Pin<&mut T> {
        self.project().inner.project().inner
    }

    pub fn into_inner(self) -> T {
        self.inner.inner
    }

    pub fn decoder(&self) -> &D {
        &self.inner.codec
    }

    pub fn decoder_mut(&mut self) -> &mut D {
        &mut self.inner.codec
    }

    pub fn read_buffer(&self) -> &BytesMut {
        &self.inner.state.buffer
    }

    pub fn read_buffer_mut(&mut self) -> &mut BytesMut {
        &mut self.inner.state.buffer
    }
}

impl<T, D> Stream for FramedRead<T, D>
where
    T: AsyncRead,
    D: Decoder,
{
    type Item = Result<D::Item, D::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().inner.poll_next(cx)
    }
}

impl<T, I, D> Sink<I> for FramedRead<T, D>
where
    T: Sink<I>,
{
    type Error = T::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().inner.project().inner.poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: I) -> Result<(), Self::Error> {
        self.project().inner.project().inner.start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().inner.project().inner.poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().inner.project().inner.poll_close(cx)
    }
}

impl<T, D> fmt::Debug for FramedRead<T, D>
where
    T: fmt::Debug,
    D: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FramedRead")
            .field("inner", &self.get_ref())
            .field("decoder", &self.decoder())
            .field("eof", &self.inner.state.eof)
            .field("is_readable", &self.inner.state.is_readable)
            .field("buffer", &self.read_buffer())
            .finish()
    }
}
