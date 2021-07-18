use bytes::{Buf, Bytes};
use cynthia::future::swap::{AsyncRead, AsyncWrite};
use futures_core::Stream;
use std::io;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use crate::proto::h2::codec::{RecvError, UserError};
use crate::proto::h2::frame::DEFAULT_INITIAL_WINDOW_SIZE;
use crate::proto::h2::frame::{Reason, StreamId};
use crate::proto::h2::proto::*;
use crate::proto::h2::{client, frame, proto, server};

#[derive(Debug)]
pub(crate) struct Connection<T, P, B: Buf = Bytes>
where
    P: Peer,
{
    state: State,

    error: Option<Reason>,

    codec: Codec<T, Prioritized<B>>,

    go_away: GoAway,

    ping_pong: PingPong,

    settings: Settings,

    streams: Streams<B, P>,

    span: tracing::Span,

    _phantom: PhantomData<P>,
}

#[derive(Debug, Clone)]
pub(crate) struct Config {
    pub next_stream_id: StreamId,
    pub initial_max_send_streams: usize,
    pub reset_stream_duration: Duration,
    pub reset_stream_max: usize,
    pub settings: frame::Settings,
}

#[derive(Debug)]
enum State {
    Open,
    Closing(Reason),
    Closed(Reason),
}

impl<T, P, B> Connection<T, P, B>
where
    T: AsyncRead + AsyncWrite + Unpin,
    P: Peer,
    B: Buf,
{
    pub fn new(codec: Codec<T, Prioritized<B>>, config: Config) -> Connection<T, P, B> {
        let streams = Streams::new(streams::Config {
            local_init_window_sz: config
                .settings
                .initial_window_size()
                .unwrap_or(DEFAULT_INITIAL_WINDOW_SIZE),
            initial_max_send_streams: config.initial_max_send_streams,
            local_next_stream_id: config.next_stream_id,
            local_push_enabled: config.settings.is_push_enabled().unwrap_or(true),
            local_reset_duration: config.reset_stream_duration,
            local_reset_max: config.reset_stream_max,
            remote_init_window_sz: DEFAULT_INITIAL_WINDOW_SIZE,
            remote_max_initiated: config
                .settings
                .max_concurrent_streams()
                .map(|max| max as usize),
        });
        Connection {
            state: State::Open,
            error: None,
            codec,
            go_away: GoAway::new(),
            ping_pong: PingPong::new(),
            settings: Settings::new(config.settings),
            streams,
            span: tracing::debug_span!("Connection", peer = %P::NAME),
            _phantom: PhantomData,
        }
    }

    pub(crate) fn set_target_window_size(&mut self, size: WindowSize) {
        self.streams.set_target_connection_window_size(size);
    }

    pub(crate) fn set_initial_window_size(&mut self, size: WindowSize) -> Result<(), UserError> {
        let mut settings = frame::Settings::default();
        settings.set_initial_window_size(Some(size));
        self.settings.send_settings(settings)
    }

    fn poll_ready(&mut self, cx: &mut Context) -> Poll<Result<(), RecvError>> {
        let _e = self.span.enter();
        let span = tracing::trace_span!("poll_ready");
        let _e = span.enter();

        ready!(self.ping_pong.send_pending_pong(cx, &mut self.codec))?;
        ready!(self.ping_pong.send_pending_ping(cx, &mut self.codec))?;
        ready!(self
            .settings
            .poll_send(cx, &mut self.codec, &mut self.streams))?;
        ready!(self.streams.send_pending_refusal(cx, &mut self.codec))?;

        Poll::Ready(Ok(()))
    }

    fn poll_go_away(&mut self, cx: &mut Context) -> Poll<Option<io::Result<Reason>>> {
        self.go_away.send_pending_go_away(cx, &mut self.codec)
    }

    fn go_away(&mut self, id: StreamId, e: Reason) {
        let frame = frame::GoAway::new(id, e);
        self.streams.send_go_away(id);
        self.go_away.go_away(frame);
    }

    fn go_away_now(&mut self, e: Reason) {
        let last_processed_id = self.streams.last_processed_id();
        let frame = frame::GoAway::new(last_processed_id, e);
        self.go_away.go_away_now(frame);
    }

    pub fn go_away_from_user(&mut self, e: Reason) {
        let last_processed_id = self.streams.last_processed_id();
        let frame = frame::GoAway::new(last_processed_id, e);
        self.go_away.go_away_from_user(frame);

        self.streams.recv_err(&proto::Error::Proto(e));
    }

    fn take_error(&mut self, ours: Reason) -> Poll<Result<(), proto::Error>> {
        let reason = if let Some(theirs) = self.error.take() {
            match (ours, theirs) {
                (Reason::NO_ERROR, err) | (err, Reason::NO_ERROR) => err,
                (_, theirs) => theirs,
            }
        } else {
            ours
        };

        if reason == Reason::NO_ERROR {
            Poll::Ready(Ok(()))
        } else {
            Poll::Ready(Err(proto::Error::Proto(reason)))
        }
    }

    pub fn maybe_close_connection_if_no_streams(&mut self) {
        if !self.streams.has_streams_or_other_references() {
            self.go_away_now(Reason::NO_ERROR);
        }
    }

    pub(crate) fn take_user_pings(&mut self) -> Option<UserPings> {
        self.ping_pong.take_user_pings()
    }

    pub fn poll(&mut self, cx: &mut Context) -> Poll<Result<(), proto::Error>> {
        let span = self.span.clone();
        let _e = span.enter();
        let span = tracing::trace_span!("poll");
        let _e = span.enter();
        use crate::proto::h2::codec::RecvError::*;

        loop {
            tracing::trace!(connection.state = ?self.state);
            match self.state {
                State::Open => match self.poll2(cx) {
                    Poll::Ready(Ok(())) => {
                        self.state = State::Closing(Reason::NO_ERROR);
                    }

                    Poll::Pending => {
                        ready!(self.streams.poll_complete(cx, &mut self.codec))?;

                        if (self.error.is_some() || self.go_away.should_close_on_idle())
                            && !self.streams.has_streams()
                        {
                            self.go_away_now(Reason::NO_ERROR);
                            continue;
                        }

                        return Poll::Pending;
                    }
                    Poll::Ready(Err(Connection(e))) => {
                        tracing::debug!(error = ?e, "Connection::poll; connection error");

                        if let Some(reason) = self.go_away.going_away_reason() {
                            if reason == e {
                                tracing::trace!("    -> already going away");
                                self.state = State::Closing(e);
                                continue;
                            }
                        }

                        self.streams.recv_err(&e.into());
                        self.go_away_now(e);
                    }
                    Poll::Ready(Err(Stream { id, reason })) => {
                        tracing::trace!(?id, ?reason, "stream error");
                        self.streams.send_reset(id, reason);
                    }
                    Poll::Ready(Err(Io(e))) => {
                        tracing::debug!(error = ?e, "Connection::poll; IO error");
                        let e = e.into();

                        self.streams.recv_err(&e);

                        return Poll::Ready(Err(e));
                    }
                },
                State::Closing(reason) => {
                    tracing::trace!("connection closing after flush");
                    ready!(self.codec.shutdown(cx))?;

                    self.state = State::Closed(reason);
                }
                State::Closed(reason) => {
                    return self.take_error(reason);
                }
            }
        }
    }

    fn poll2(&mut self, cx: &mut Context) -> Poll<Result<(), RecvError>> {
        use crate::proto::h2::frame::Frame::*;

        self.clear_expired_reset_streams();

        loop {
            if let Some(reason) = ready!(self.poll_go_away(cx)?) {
                if self.go_away.should_close_now() {
                    if self.go_away.is_user_initiated() {
                        return Poll::Ready(Ok(()));
                    } else {
                        return Poll::Ready(Err(RecvError::Connection(reason)));
                    }
                }
                debug_assert_eq!(
                    reason,
                    Reason::NO_ERROR,
                    "graceful GOAWAY should be NO_ERROR"
                );
            }
            ready!(self.poll_ready(cx))?;

            match ready!(Pin::new(&mut self.codec).poll_next(cx)?) {
                Some(Headers(frame)) => {
                    tracing::trace!(?frame, "recv HEADERS");
                    self.streams.recv_headers(frame)?;
                }
                Some(Data(frame)) => {
                    tracing::trace!(?frame, "recv DATA");
                    self.streams.recv_data(frame)?;
                }
                Some(Reset(frame)) => {
                    tracing::trace!(?frame, "recv RST_STREAM");
                    self.streams.recv_reset(frame)?;
                }
                Some(PushPromise(frame)) => {
                    tracing::trace!(?frame, "recv PUSH_PROMISE");
                    self.streams.recv_push_promise(frame)?;
                }
                Some(Settings(frame)) => {
                    tracing::trace!(?frame, "recv SETTINGS");
                    self.settings
                        .recv_settings(frame, &mut self.codec, &mut self.streams)?;
                }
                Some(GoAway(frame)) => {
                    tracing::trace!(?frame, "recv GOAWAY");
                    self.streams.recv_go_away(&frame)?;
                    self.error = Some(frame.reason());
                }
                Some(Ping(frame)) => {
                    tracing::trace!(?frame, "recv PING");
                    let status = self.ping_pong.recv_ping(frame);
                    if status.is_shutdown() {
                        assert!(
                            self.go_away.is_going_away(),
                            "received unexpected shutdown ping"
                        );

                        let last_processed_id = self.streams.last_processed_id();
                        self.go_away(last_processed_id, Reason::NO_ERROR);
                    }
                }
                Some(WindowUpdate(frame)) => {
                    tracing::trace!(?frame, "recv WINDOW_UPDATE");
                    self.streams.recv_window_update(frame)?;
                }
                Some(Priority(frame)) => {
                    tracing::trace!(?frame, "recv PRIORITY");
                }
                None => {
                    tracing::trace!("codec closed");
                    self.streams.recv_eof(false).expect("mutex poisoned");
                    return Poll::Ready(Ok(()));
                }
            }
        }
    }

    fn clear_expired_reset_streams(&mut self) {
        self.streams.clear_expired_reset_streams();
    }
}

impl<T, B> Connection<T, client::Peer, B>
where
    T: AsyncRead + AsyncWrite,
    B: Buf,
{
    pub(crate) fn streams(&self) -> &Streams<B, client::Peer> {
        &self.streams
    }
}

impl<T, B> Connection<T, server::Peer, B>
where
    T: AsyncRead + AsyncWrite + Unpin,
    B: Buf,
{
    pub fn next_incoming(&mut self) -> Option<StreamRef<B>> {
        self.streams.next_incoming()
    }

    pub fn go_away_gracefully(&mut self) {
        if self.go_away.is_going_away() {
            return;
        }

        self.go_away(StreamId::MAX, Reason::NO_ERROR);

        self.ping_pong.ping_shutdown();
    }
}

impl<T, P, B> Drop for Connection<T, P, B>
where
    P: Peer,
    B: Buf,
{
    fn drop(&mut self) {
        let _ = self.streams.recv_eof(true);
    }
}
