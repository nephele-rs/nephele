use bytes::BytesMut;
use cynthia::future::swap::AsyncWrite;
use futures_core::stream::Stream;
use futures_sink::Sink;
use pin_project_lite::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{fmt, io};

use crate::common::codec::encoder::Encoder;
use crate::common::codec::framed_impl::{FramedImpl, WriteFrame};

pin_project! {
    pub struct FramedWrite<T, E> {
        #[pin]
        inner: FramedImpl<T, E, WriteFrame>,
    }
}

impl<T, E> FramedWrite<T, E>
where
    T: AsyncWrite,
{
    pub fn new(inner: T, encoder: E) -> FramedWrite<T, E> {
        FramedWrite {
            inner: FramedImpl {
                inner,
                codec: encoder,
                state: WriteFrame::default(),
            },
        }
    }
}

impl<T, E> FramedWrite<T, E> {
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

    pub fn encoder(&self) -> &E {
        &self.inner.codec
    }

    pub fn encoder_mut(&mut self) -> &mut E {
        &mut self.inner.codec
    }

    pub fn write_buffer(&self) -> &BytesMut {
        &self.inner.state.buffer
    }

    pub fn write_buffer_mut(&mut self) -> &mut BytesMut {
        &mut self.inner.state.buffer
    }
}

impl<T, I, E> Sink<I> for FramedWrite<T, E>
where
    T: AsyncWrite,
    E: Encoder<I>,
    E::Error: From<io::Error>,
{
    type Error = E::Error;

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

impl<T, D> Stream for FramedWrite<T, D>
where
    T: Stream,
{
    type Item = T::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().inner.project().inner.poll_next(cx)
    }
}

impl<T, U> fmt::Debug for FramedWrite<T, U>
where
    T: fmt::Debug,
    U: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FramedWrite")
            .field("inner", &self.get_ref())
            .field("encoder", &self.encoder())
            .field("buffer", &self.inner.state.buffer)
            .finish()
    }
}
