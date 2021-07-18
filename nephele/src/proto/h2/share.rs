use bytes::{Buf, Bytes};
use http::HeaderMap;
use std::fmt;
#[cfg(feature = "stream")]
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::proto::h2::codec::UserError;
use crate::proto::h2::frame::Reason;
use crate::proto::h2::proto::{self, WindowSize};
use crate::proto::h2::PollExt;

#[derive(Debug)]
pub struct SendStream<B: Buf> {
    inner: proto::StreamRef<B>,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct StreamId(u32);

#[must_use = "streams do nothing unless polled"]
pub struct RecvStream {
    inner: FlowControl,
}

#[derive(Clone, Debug)]
pub struct FlowControl {
    inner: proto::OpaqueStreamRef,
}

pub struct PingPong {
    inner: proto::UserPings,
}

pub struct Ping {
    _p: (),
}

pub struct Pong {
    _p: (),
}

impl<B: Buf> SendStream<B> {
    pub(crate) fn new(inner: proto::StreamRef<B>) -> Self {
        SendStream { inner }
    }

    pub fn reserve_capacity(&mut self, capacity: usize) {
        self.inner.reserve_capacity(capacity as WindowSize)
    }

    pub fn capacity(&self) -> usize {
        self.inner.capacity() as usize
    }

    pub fn poll_capacity(
        &mut self,
        cx: &mut Context,
    ) -> Poll<Option<Result<usize, crate::proto::h2::Error>>> {
        self.inner
            .poll_capacity(cx)
            .map_ok_(|w| w as usize)
            .map_err_(Into::into)
    }

    pub fn send_data(
        &mut self,
        data: B,
        end_of_stream: bool,
    ) -> Result<(), crate::proto::h2::Error> {
        self.inner
            .send_data(data, end_of_stream)
            .map_err(Into::into)
    }

    pub fn send_trailers(&mut self, trailers: HeaderMap) -> Result<(), crate::proto::h2::Error> {
        self.inner.send_trailers(trailers).map_err(Into::into)
    }

    pub fn send_reset(&mut self, reason: Reason) {
        self.inner.send_reset(reason)
    }

    pub fn poll_reset(
        &mut self,
        cx: &mut Context,
    ) -> Poll<Result<Reason, crate::proto::h2::Error>> {
        self.inner.poll_reset(cx, proto::PollReset::Streaming)
    }

    pub fn stream_id(&self) -> StreamId {
        StreamId::from_internal(self.inner.stream_id())
    }
}

impl StreamId {
    pub(crate) fn from_internal(id: crate::proto::h2::frame::StreamId) -> Self {
        StreamId(id.into())
    }
}

impl RecvStream {
    pub(crate) fn new(inner: FlowControl) -> Self {
        RecvStream { inner }
    }

    pub async fn data(&mut self) -> Option<Result<Bytes, crate::proto::h2::Error>> {
        futures_util::future::poll_fn(move |cx| self.poll_data(cx)).await
    }

    pub async fn trailers(&mut self) -> Result<Option<HeaderMap>, crate::proto::h2::Error> {
        futures_util::future::poll_fn(move |cx| self.poll_trailers(cx)).await
    }

    pub fn poll_data(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, crate::proto::h2::Error>>> {
        self.inner.inner.poll_data(cx).map_err_(Into::into)
    }

    pub fn poll_trailers(
        &mut self,
        cx: &mut Context,
    ) -> Poll<Result<Option<HeaderMap>, crate::proto::h2::Error>> {
        match ready!(self.inner.inner.poll_trailers(cx)) {
            Some(Ok(map)) => Poll::Ready(Ok(Some(map))),
            Some(Err(e)) => Poll::Ready(Err(e.into())),
            None => Poll::Ready(Ok(None)),
        }
    }

    pub fn is_end_stream(&self) -> bool {
        self.inner.inner.is_end_stream()
    }

    pub fn flow_control(&mut self) -> &mut FlowControl {
        &mut self.inner
    }

    pub fn stream_id(&self) -> StreamId {
        self.inner.stream_id()
    }
}

#[cfg(feature = "stream")]
impl futures_core::Stream for RecvStream {
    type Item = Result<Bytes, crate::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.poll_data(cx)
    }
}

impl fmt::Debug for RecvStream {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("RecvStream")
            .field("inner", &self.inner)
            .finish()
    }
}

impl Drop for RecvStream {
    fn drop(&mut self) {
        self.inner.inner.clear_recv_buffer();
    }
}

impl FlowControl {
    pub(crate) fn new(inner: proto::OpaqueStreamRef) -> Self {
        FlowControl { inner }
    }

    pub fn stream_id(&self) -> StreamId {
        StreamId::from_internal(self.inner.stream_id())
    }

    pub fn available_capacity(&self) -> isize {
        self.inner.available_recv_capacity()
    }

    pub fn used_capacity(&self) -> usize {
        self.inner.used_recv_capacity() as usize
    }

    pub fn release_capacity(&mut self, sz: usize) -> Result<(), crate::proto::h2::Error> {
        if sz > proto::MAX_WINDOW_SIZE as usize {
            return Err(UserError::ReleaseCapacityTooBig.into());
        }
        self.inner
            .release_capacity(sz as proto::WindowSize)
            .map_err(Into::into)
    }
}

impl PingPong {
    pub(crate) fn new(inner: proto::UserPings) -> Self {
        PingPong { inner }
    }

    pub async fn ping(&mut self, ping: Ping) -> Result<Pong, crate::proto::h2::Error> {
        self.send_ping(ping)?;
        futures_util::future::poll_fn(|cx| self.poll_pong(cx)).await
    }

    #[doc(hidden)]
    pub fn send_ping(&mut self, ping: Ping) -> Result<(), crate::proto::h2::Error> {
        drop(ping);

        self.inner.send_ping().map_err(|err| match err {
            Some(err) => err.into(),
            None => UserError::SendPingWhilePending.into(),
        })
    }

    #[doc(hidden)]
    pub fn poll_pong(&mut self, cx: &mut Context) -> Poll<Result<Pong, crate::proto::h2::Error>> {
        ready!(self.inner.poll_pong(cx))?;
        Poll::Ready(Ok(Pong { _p: () }))
    }
}

impl fmt::Debug for PingPong {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("PingPong").finish()
    }
}

impl Ping {
    pub fn opaque() -> Ping {
        Ping { _p: () }
    }
}

impl fmt::Debug for Ping {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("Ping").finish()
    }
}

impl fmt::Debug for Pong {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("Pong").finish()
    }
}
