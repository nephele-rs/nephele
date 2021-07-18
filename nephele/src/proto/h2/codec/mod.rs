mod error;
mod framed_read;
mod framed_write;

use bytes::Buf;
use cynthia::future::swap::{AsyncRead, AsyncWrite};
use futures_core::Stream;
use futures_sink::Sink;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use self::framed_read::FramedRead;
use self::framed_write::FramedWrite;
use crate::common::codec::length_delimited;
pub use crate::proto::h2::codec::error::{RecvError, SendError, UserError};
use crate::proto::h2::frame::{self, Data, Frame};

#[derive(Debug)]
pub struct Codec<T, B> {
    inner: FramedRead<FramedWrite<T, B>>,
}

impl<T, B> Codec<T, B>
where
    T: AsyncRead + AsyncWrite + Unpin,
    B: Buf,
{
    #[inline]
    pub fn new(io: T) -> Self {
        Self::with_max_recv_frame_size(io, frame::DEFAULT_MAX_FRAME_SIZE as usize)
    }

    pub fn with_max_recv_frame_size(io: T, max_frame_size: usize) -> Self {
        let framed_write = FramedWrite::new(io);

        let delimited = length_delimited::Builder::new()
            .big_endian()
            .length_field_length(3)
            .length_adjustment(9)
            .num_skip(0)
            .new_read(framed_write);

        let mut inner = FramedRead::new(delimited);

        inner.set_max_frame_size(max_frame_size);

        Codec { inner }
    }
}

impl<T, B> Codec<T, B> {
    #[inline]
    pub fn set_max_recv_frame_size(&mut self, val: usize) {
        self.inner.set_max_frame_size(val)
    }

    #[cfg(feature = "unstable")]
    #[inline]
    pub fn max_recv_frame_size(&self) -> usize {
        self.inner.max_frame_size()
    }

    pub fn max_send_frame_size(&self) -> usize {
        self.inner.get_ref().max_frame_size()
    }

    pub fn set_max_send_frame_size(&mut self, val: usize) {
        self.framed_write().set_max_frame_size(val)
    }

    pub fn set_send_header_table_size(&mut self, val: usize) {
        self.framed_write().set_header_table_size(val)
    }

    pub fn set_max_recv_header_list_size(&mut self, val: usize) {
        self.inner.set_max_header_list_size(val);
    }

    #[cfg(feature = "unstable")]
    pub fn get_ref(&self) -> &T {
        self.inner.get_ref().get_ref()
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.inner.get_mut().get_mut()
    }

    pub(crate) fn take_last_data_frame(&mut self) -> Option<Data<B>> {
        self.framed_write().take_last_data_frame()
    }

    fn framed_write(&mut self) -> &mut FramedWrite<T, B> {
        self.inner.get_mut()
    }
}

impl<T, B> Codec<T, B>
where
    T: AsyncWrite + Unpin,
    B: Buf,
{
    pub fn poll_ready(&mut self, cx: &mut Context) -> Poll<io::Result<()>> {
        self.framed_write().poll_ready(cx)
    }

    pub fn buffer(&mut self, item: Frame<B>) -> Result<(), UserError> {
        self.framed_write().buffer(item)
    }

    pub fn flush(&mut self, cx: &mut Context) -> Poll<io::Result<()>> {
        self.framed_write().flush(cx)
    }

    pub fn shutdown(&mut self, cx: &mut Context) -> Poll<io::Result<()>> {
        self.framed_write().shutdown(cx)
    }
}

impl<T, B> Stream for Codec<T, B>
where
    T: AsyncRead + Unpin,
{
    type Item = Result<Frame, RecvError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

impl<T, B> Sink<Frame<B>> for Codec<T, B>
where
    T: AsyncWrite + Unpin,
    B: Buf,
{
    type Error = SendError;

    fn start_send(mut self: Pin<&mut Self>, item: Frame<B>) -> Result<(), Self::Error> {
        Codec::buffer(&mut self, item)?;
        Ok(())
    }
    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.framed_write().poll_ready(cx).map_err(Into::into)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.framed_write().flush(cx).map_err(Into::into)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self.shutdown(cx))?;
        Poll::Ready(Ok(()))
    }
}

impl<T> From<T> for Codec<T, bytes::Bytes>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    fn from(src: T) -> Self {
        Self::new(src)
    }
}
