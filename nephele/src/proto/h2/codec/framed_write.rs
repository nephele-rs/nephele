use bytes::{Buf, BufMut, BytesMut};
use cynthia::future::swap::{AsyncRead, AsyncWrite};
use std::io::{self, Cursor, IoSlice};
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::proto::h2::codec::UserError;
use crate::proto::h2::codec::UserError::*;
use crate::proto::h2::frame::{self, Frame, FrameSize};
use crate::proto::h2::hpack;

macro_rules! limited_write_buf {
    ($self:expr) => {{
        let limit = $self.max_frame_size() + frame::HEADER_LEN;
        $self.buf.get_mut().limit(limit)
    }};
}

#[derive(Debug)]
pub struct FramedWrite<T, B> {
    inner: T,
    hpack: hpack::Encoder,
    buf: Cursor<BytesMut>,
    next: Option<Next<B>>,
    last_data_frame: Option<frame::Data<B>>,
    max_frame_size: FrameSize,
    is_write_vectored: bool,
}

#[derive(Debug)]
enum Next<B> {
    Data(frame::Data<B>),
    Continuation(frame::Continuation),
}

const DEFAULT_BUFFER_CAPACITY: usize = 16 * 1_024;

const MIN_BUFFER_CAPACITY: usize = frame::HEADER_LEN + CHAIN_THRESHOLD;

const CHAIN_THRESHOLD: usize = 256;

impl<T, B> FramedWrite<T, B>
where
    T: AsyncWrite + Unpin,
    B: Buf,
{
    pub fn new(inner: T) -> FramedWrite<T, B> {
        let is_write_vectored = false;
        FramedWrite {
            inner,
            hpack: hpack::Encoder::default(),
            buf: Cursor::new(BytesMut::with_capacity(DEFAULT_BUFFER_CAPACITY)),
            next: None,
            last_data_frame: None,
            max_frame_size: frame::DEFAULT_MAX_FRAME_SIZE,
            is_write_vectored,
        }
    }

    pub fn poll_ready(&mut self, cx: &mut Context) -> Poll<io::Result<()>> {
        if !self.has_capacity() {
            ready!(self.flush(cx))?;

            if !self.has_capacity() {
                return Poll::Pending;
            }
        }

        Poll::Ready(Ok(()))
    }

    pub fn buffer(&mut self, item: Frame<B>) -> Result<(), UserError> {
        assert!(self.has_capacity());
        let span = tracing::trace_span!("FramedWrite::buffer", frame = ?item);
        let _e = span.enter();

        tracing::debug!(frame = ?item, "send");

        match item {
            Frame::Data(mut v) => {
                let len = v.payload().remaining();

                if len > self.max_frame_size() {
                    return Err(PayloadTooBig);
                }

                if len >= CHAIN_THRESHOLD {
                    let head = v.head();

                    head.encode(len, self.buf.get_mut());

                    self.next = Some(Next::Data(v));
                } else {
                    v.encode_chunk(self.buf.get_mut());

                    assert_eq!(v.payload().remaining(), 0, "chunk not fully encoded");

                    self.last_data_frame = Some(v);
                }
            }
            Frame::Headers(v) => {
                let mut buf = limited_write_buf!(self);
                if let Some(continuation) = v.encode(&mut self.hpack, &mut buf) {
                    self.next = Some(Next::Continuation(continuation));
                }
            }
            Frame::PushPromise(v) => {
                let mut buf = limited_write_buf!(self);
                if let Some(continuation) = v.encode(&mut self.hpack, &mut buf) {
                    self.next = Some(Next::Continuation(continuation));
                }
            }
            Frame::Settings(v) => {
                v.encode(self.buf.get_mut());
                tracing::trace!(rem = self.buf.remaining(), "encoded settings");
            }
            Frame::GoAway(v) => {
                v.encode(self.buf.get_mut());
                tracing::trace!(rem = self.buf.remaining(), "encoded go_away");
            }
            Frame::Ping(v) => {
                v.encode(self.buf.get_mut());
                tracing::trace!(rem = self.buf.remaining(), "encoded ping");
            }
            Frame::WindowUpdate(v) => {
                v.encode(self.buf.get_mut());
                tracing::trace!(rem = self.buf.remaining(), "encoded window_update");
            }

            Frame::Priority(_) => {
                unimplemented!();
            }
            Frame::Reset(v) => {
                v.encode(self.buf.get_mut());
                tracing::trace!(rem = self.buf.remaining(), "encoded reset");
            }
        }

        Ok(())
    }

    pub fn flush(&mut self, cx: &mut Context) -> Poll<io::Result<()>> {
        const MAX_IOVS: usize = 64;

        let span = tracing::trace_span!("FramedWrite::flush");
        let _e = span.enter();

        loop {
            while !self.is_empty() {
                match self.next {
                    Some(Next::Data(ref mut frame)) => {
                        tracing::trace!(queued_data_frame = true);
                        let mut buf = (&mut self.buf).chain(frame.payload_mut());
                        let n = if self.is_write_vectored {
                            let mut bufs = [IoSlice::new(&[]); MAX_IOVS];
                            let cnt = buf.chunks_vectored(&mut bufs);
                            ready!(Pin::new(&mut self.inner).poll_write_vectored(cx, &bufs[..cnt]))?
                        } else {
                            ready!(Pin::new(&mut self.inner).poll_write(cx, buf.chunk()))?
                        };
                        buf.advance(n);
                    }
                    _ => {
                        tracing::trace!(queued_data_frame = false);
                        let n = if self.is_write_vectored {
                            let mut iovs = [IoSlice::new(&[]); MAX_IOVS];
                            let cnt = self.buf.chunks_vectored(&mut iovs);
                            ready!(
                                Pin::new(&mut self.inner).poll_write_vectored(cx, &mut iovs[..cnt])
                            )?
                        } else {
                            ready!(Pin::new(&mut self.inner).poll_write(cx, &mut self.buf.chunk()))?
                        };
                        self.buf.advance(n);
                    }
                }
            }

            self.buf.set_position(0);
            self.buf.get_mut().clear();

            match self.next.take() {
                Some(Next::Data(frame)) => {
                    self.last_data_frame = Some(frame);
                    debug_assert!(self.is_empty());
                    break;
                }
                Some(Next::Continuation(frame)) => {
                    let mut buf = limited_write_buf!(self);
                    if let Some(continuation) = frame.encode(&mut self.hpack, &mut buf) {
                        if self.buf.get_ref().len() == frame::HEADER_LEN {
                            panic!("CONTINUATION frame write loop; header value too big to encode");
                        }

                        self.next = Some(Next::Continuation(continuation));
                    }
                }
                None => {
                    break;
                }
            }
        }

        tracing::trace!("flushing buffer");
        ready!(Pin::new(&mut self.inner).poll_flush(cx))?;

        Poll::Ready(Ok(()))
    }

    pub fn shutdown(&mut self, cx: &mut Context) -> Poll<io::Result<()>> {
        ready!(self.flush(cx))?;
        Pin::new(&mut self.inner).poll_close(cx)
    }

    fn has_capacity(&self) -> bool {
        self.next.is_none() && self.buf.get_ref().remaining_mut() >= MIN_BUFFER_CAPACITY
    }

    fn is_empty(&self) -> bool {
        match self.next {
            Some(Next::Data(ref frame)) => !frame.payload().has_remaining(),
            _ => !self.buf.has_remaining(),
        }
    }
}

impl<T, B> FramedWrite<T, B> {
    pub fn max_frame_size(&self) -> usize {
        self.max_frame_size as usize
    }

    pub fn set_max_frame_size(&mut self, val: usize) {
        assert!(val <= frame::MAX_MAX_FRAME_SIZE as usize);
        self.max_frame_size = val as FrameSize;
    }

    pub fn set_header_table_size(&mut self, val: usize) {
        self.hpack.update_max_size(val);
    }

    pub fn take_last_data_frame(&mut self) -> Option<frame::Data<B>> {
        self.last_data_frame.take()
    }

    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T: AsyncRead + Unpin, B> AsyncRead for FramedWrite<T, B> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<T: Unpin, B> Unpin for FramedWrite<T, B> {}

#[cfg(feature = "unstable")]
mod unstable {
    use super::*;

    impl<T, B> FramedWrite<T, B> {
        pub fn get_ref(&self) -> &T {
            &self.inner
        }
    }
}
