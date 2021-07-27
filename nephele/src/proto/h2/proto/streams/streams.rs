use bytes::{Buf, Bytes};
use cynthia::future::swap::AsyncWrite;
use http::{HeaderMap, Request, Response};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::{fmt, io};

use crate::proto::h2::codec::{Codec, RecvError, SendError, UserError};
use crate::proto::h2::frame::{self, Frame, Reason};
use crate::proto::h2::proto::streams::recv::RecvHeaderBlockError;
use crate::proto::h2::proto::streams::store::{self, Entry, Resolve, Store};
use crate::proto::h2::proto::streams::{
    Buffer, Config, Counts, Prioritized, Recv, Send, Stream, StreamId,
};
use crate::proto::h2::proto::{peer, Open, Peer, WindowSize};
use crate::proto::h2::PollExt;
use crate::proto::h2::{client, proto, server};

#[derive(Debug)]
pub(crate) struct Streams<B, P>
where
    P: Peer,
{
    inner: Arc<Mutex<Inner>>,

    send_buffer: Arc<SendBuffer<B>>,

    _p: ::std::marker::PhantomData<P>,
}

#[derive(Debug)]
pub(crate) struct StreamRef<B> {
    opaque: OpaqueStreamRef,
    send_buffer: Arc<SendBuffer<B>>,
}

pub(crate) struct OpaqueStreamRef {
    inner: Arc<Mutex<Inner>>,
    key: store::Key,
}

#[derive(Debug)]
struct Inner {
    counts: Counts,
    actions: Actions,
    store: Store,
    refs: usize,
}

#[derive(Debug)]
struct Actions {
    recv: Recv,
    send: Send,
    task: Option<Waker>,
    conn_error: Option<proto::Error>,
}

#[derive(Debug)]
struct SendBuffer<B> {
    inner: Mutex<Buffer<Frame<B>>>,
}

impl<B, P> Streams<B, P>
where
    B: Buf,
    P: Peer,
{
    pub fn new(config: Config) -> Self {
        let peer = P::r#dyn();

        Streams {
            inner: Arc::new(Mutex::new(Inner {
                counts: Counts::new(peer, &config),
                actions: Actions {
                    recv: Recv::new(peer, &config),
                    send: Send::new(&config),
                    task: None,
                    conn_error: None,
                },
                store: Store::new(),
                refs: 1,
            })),
            send_buffer: Arc::new(SendBuffer::new()),
            _p: ::std::marker::PhantomData,
        }
    }

    pub fn set_target_connection_window_size(&mut self, size: WindowSize) {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        me.actions
            .recv
            .set_target_connection_window(size, &mut me.actions.task)
    }

    pub fn recv_headers(&mut self, frame: frame::Headers) -> Result<(), RecvError> {
        let id = frame.stream_id();
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        if id > me.actions.recv.max_stream_id() {
            tracing::trace!(
                "id ({:?}) > max_stream_id ({:?}), ignoring HEADERS",
                id,
                me.actions.recv.max_stream_id()
            );
            return Ok(());
        }

        let key = match me.store.find_entry(id) {
            Entry::Occupied(e) => e.key(),
            Entry::Vacant(e) => {
                if !P::is_server() {
                    if me.actions.may_have_forgotten_stream::<P>(id) {
                        tracing::debug!(
                            "recv_headers for old stream={:?}, sending STREAM_CLOSED",
                            id,
                        );
                        return Err(RecvError::Stream {
                            id,
                            reason: Reason::STREAM_CLOSED,
                        });
                    }
                }

                match me.actions.recv.open(id, Open::Headers, &mut me.counts)? {
                    Some(stream_id) => {
                        let stream = Stream::new(
                            stream_id,
                            me.actions.send.init_window_sz(),
                            me.actions.recv.init_window_sz(),
                        );

                        e.insert(stream)
                    }
                    None => return Ok(()),
                }
            }
        };

        let stream = me.store.resolve(key);

        if stream.state.is_local_reset() {
            tracing::trace!("recv_headers; ignoring trailers on {:?}", stream.id);
            return Ok(());
        }

        let actions = &mut me.actions;
        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        me.counts.transition(stream, |counts, stream| {
            tracing::trace!(
                "recv_headers; stream={:?}; state={:?}",
                stream.id,
                stream.state
            );

            let res = if stream.state.is_recv_headers() {
                match actions.recv.recv_headers(frame, stream, counts) {
                    Ok(()) => Ok(()),
                    Err(RecvHeaderBlockError::Oversize(resp)) => {
                        if let Some(resp) = resp {
                            let sent = actions.send.send_headers(
                                resp, send_buffer, stream, counts, &mut actions.task);
                            debug_assert!(sent.is_ok(), "oversize response should not fail");
                            actions.send.schedule_implicit_reset(
                                stream,
                                Reason::REFUSED_STREAM,
                                counts,
                                &mut actions.task);

                            actions.recv.enqueue_reset_expiration(stream, counts);

                            Ok(())
                        } else {
                            Err(RecvError::Stream {
                                id: stream.id,
                                reason: Reason::REFUSED_STREAM,
                            })
                        }
                    },
                    Err(RecvHeaderBlockError::State(err)) => Err(err),
                }
            } else {
                if !frame.is_end_stream() {
                    proto_err!(stream: "recv_headers: trailers frame was not EOS; stream={:?}", stream.id);
                    return Err(RecvError::Stream {
                        id: stream.id,
                        reason: Reason::PROTOCOL_ERROR,
                    });
                }

                actions.recv.recv_trailers(frame, stream)
            };

            actions.reset_on_recv_stream_err(send_buffer, stream, counts, res)
        })
    }

    pub fn recv_data(&mut self, frame: frame::Data) -> Result<(), RecvError> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        let id = frame.stream_id();

        let stream = match me.store.find_mut(&id) {
            Some(stream) => stream,
            None => {
                if id > me.actions.recv.max_stream_id() {
                    tracing::trace!(
                        "id ({:?}) > max_stream_id ({:?}), ignoring DATA",
                        id,
                        me.actions.recv.max_stream_id()
                    );
                    return Ok(());
                }

                if me.actions.may_have_forgotten_stream::<P>(id) {
                    tracing::debug!("recv_data for old stream={:?}, sending STREAM_CLOSED", id,);

                    let sz = frame.payload().len();
                    assert!(sz <= super::MAX_WINDOW_SIZE as usize);
                    let sz = sz as WindowSize;

                    me.actions.recv.ignore_data(sz)?;
                    return Err(RecvError::Stream {
                        id,
                        reason: Reason::STREAM_CLOSED,
                    });
                }

                proto_err!(conn: "recv_data: stream not found; id={:?}", id);
                return Err(RecvError::Connection(Reason::PROTOCOL_ERROR));
            }
        };

        let actions = &mut me.actions;
        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        me.counts.transition(stream, |counts, stream| {
            let sz = frame.payload().len();
            let res = actions.recv.recv_data(frame, stream);

            if let Err(RecvError::Stream { .. }) = res {
                actions
                    .recv
                    .release_connection_capacity(sz as WindowSize, &mut None);
            }
            actions.reset_on_recv_stream_err(send_buffer, stream, counts, res)
        })
    }

    pub fn recv_reset(&mut self, frame: frame::Reset) -> Result<(), RecvError> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        let id = frame.stream_id();

        if id.is_zero() {
            proto_err!(conn: "recv_reset: invalid stream ID 0");
            return Err(RecvError::Connection(Reason::PROTOCOL_ERROR));
        }

        if id > me.actions.recv.max_stream_id() {
            tracing::trace!(
                "id ({:?}) > max_stream_id ({:?}), ignoring RST_STREAM",
                id,
                me.actions.recv.max_stream_id()
            );
            return Ok(());
        }

        let stream = match me.store.find_mut(&id) {
            Some(stream) => stream,
            None => {
                me.actions
                    .ensure_not_idle(me.counts.peer(), id)
                    .map_err(RecvError::Connection)?;

                return Ok(());
            }
        };

        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        let actions = &mut me.actions;

        me.counts.transition(stream, |counts, stream| {
            actions.recv.recv_reset(frame, stream);
            actions.send.recv_err(send_buffer, stream, counts);
            assert!(stream.state.is_closed());
            Ok(())
        })
    }

    pub fn recv_err(&mut self, err: &proto::Error) -> StreamId {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        let actions = &mut me.actions;
        let counts = &mut me.counts;
        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        let last_processed_id = actions.recv.last_processed_id();

        me.store
            .for_each(|stream| {
                counts.transition(stream, |counts, stream| {
                    actions.recv.recv_err(err, &mut *stream);
                    actions.send.recv_err(send_buffer, stream, counts);
                    Ok::<_, ()>(())
                })
            })
            .unwrap();

        actions.conn_error = Some(err.shallow_clone());

        last_processed_id
    }

    pub fn recv_go_away(&mut self, frame: &frame::GoAway) -> Result<(), RecvError> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        let actions = &mut me.actions;
        let counts = &mut me.counts;
        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        let last_stream_id = frame.last_stream_id();

        actions.send.recv_go_away(last_stream_id)?;

        let err = frame.reason().into();

        me.store
            .for_each(|stream| {
                if stream.id > last_stream_id {
                    counts.transition(stream, |counts, stream| {
                        actions.recv.recv_err(&err, &mut *stream);
                        actions.send.recv_err(send_buffer, stream, counts);
                        Ok::<_, ()>(())
                    })
                } else {
                    Ok::<_, ()>(())
                }
            })
            .unwrap();

        actions.conn_error = Some(err);

        Ok(())
    }

    pub fn last_processed_id(&self) -> StreamId {
        self.inner.lock().unwrap().actions.recv.last_processed_id()
    }

    pub fn recv_window_update(&mut self, frame: frame::WindowUpdate) -> Result<(), RecvError> {
        let id = frame.stream_id();
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        if id.is_zero() {
            me.actions
                .send
                .recv_connection_window_update(frame, &mut me.store, &mut me.counts)
                .map_err(RecvError::Connection)?;
        } else {
            if let Some(mut stream) = me.store.find_mut(&id) {
                let _ = me.actions.send.recv_stream_window_update(
                    frame.size_increment(),
                    send_buffer,
                    &mut stream,
                    &mut me.counts,
                    &mut me.actions.task,
                );
            } else {
                me.actions
                    .ensure_not_idle(me.counts.peer(), id)
                    .map_err(RecvError::Connection)?;
            }
        }

        Ok(())
    }

    pub fn recv_push_promise(&mut self, frame: frame::PushPromise) -> Result<(), RecvError> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        let id = frame.stream_id();
        let promised_id = frame.promised_id();

        let parent_key = match me.store.find_mut(&id) {
            Some(stream) => {
                if id > me.actions.recv.max_stream_id() {
                    tracing::trace!(
                        "id ({:?}) > max_stream_id ({:?}), ignoring PUSH_PROMISE",
                        id,
                        me.actions.recv.max_stream_id()
                    );
                    return Ok(());
                }

                stream.state.ensure_recv_open()?;
                stream.key()
            }
            None => {
                proto_err!(conn: "recv_push_promise: initiating stream is in an invalid state");
                return Err(RecvError::Connection(Reason::PROTOCOL_ERROR));
            }
        };

        me.actions.recv.ensure_can_reserve()?;

        if me
            .actions
            .recv
            .open(promised_id, Open::PushPromise, &mut me.counts)?
            .is_none()
        {
            return Ok(());
        }

        let child_key: Option<store::Key> = {
            let stream = me.store.insert(promised_id, {
                Stream::new(
                    promised_id,
                    me.actions.send.init_window_sz(),
                    me.actions.recv.init_window_sz(),
                )
            });

            let actions = &mut me.actions;

            me.counts.transition(stream, |counts, stream| {
                let stream_valid = actions.recv.recv_push_promise(frame, stream);

                match stream_valid {
                    Ok(()) => Ok(Some(stream.key())),
                    _ => {
                        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
                        actions
                            .reset_on_recv_stream_err(
                                &mut *send_buffer,
                                stream,
                                counts,
                                stream_valid,
                            )
                            .map(|()| None)
                    }
                }
            })?
        };
        if let Some(child) = child_key {
            let mut ppp = me.store[parent_key].pending_push_promises.take();
            ppp.push(&mut me.store.resolve(child));

            let parent = &mut me.store.resolve(parent_key);
            parent.pending_push_promises = ppp;
            parent.notify_recv();
        };

        Ok(())
    }

    pub fn next_incoming(&mut self) -> Option<StreamRef<B>> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;
        me.actions.recv.next_incoming(&mut me.store).map(|key| {
            let stream = &mut me.store.resolve(key);
            tracing::trace!(
                "next_incoming; id={:?}, state={:?}",
                stream.id,
                stream.state
            );
            me.refs += 1;
            StreamRef {
                opaque: OpaqueStreamRef::new(self.inner.clone(), stream),
                send_buffer: self.send_buffer.clone(),
            }
        })
    }

    pub fn send_pending_refusal<T>(
        &mut self,
        cx: &mut Context,
        dst: &mut Codec<T, Prioritized<B>>,
    ) -> Poll<io::Result<()>>
    where
        T: AsyncWrite + Unpin,
    {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;
        me.actions.recv.send_pending_refusal(cx, dst)
    }

    pub fn clear_expired_reset_streams(&mut self) {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;
        me.actions
            .recv
            .clear_expired_reset_streams(&mut me.store, &mut me.counts);
    }

    pub fn poll_complete<T>(
        &mut self,
        cx: &mut Context,
        dst: &mut Codec<T, Prioritized<B>>,
    ) -> Poll<io::Result<()>>
    where
        T: AsyncWrite + Unpin,
    {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        ready!(me
            .actions
            .recv
            .poll_complete(cx, &mut me.store, &mut me.counts, dst))?;

        ready!(me
            .actions
            .send
            .poll_complete(cx, send_buffer, &mut me.store, &mut me.counts, dst))?;

        me.actions.task = Some(cx.waker().clone());

        Poll::Ready(Ok(()))
    }

    pub fn apply_remote_settings(&mut self, frame: &frame::Settings) -> Result<(), RecvError> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        me.counts.apply_remote_settings(frame);

        me.actions.send.apply_remote_settings(
            frame,
            send_buffer,
            &mut me.store,
            &mut me.counts,
            &mut me.actions.task,
        )
    }

    pub fn apply_local_settings(&mut self, frame: &frame::Settings) -> Result<(), RecvError> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        me.actions.recv.apply_local_settings(frame, &mut me.store)
    }

    pub fn send_request(
        &mut self,
        request: Request<()>,
        end_of_stream: bool,
        pending: Option<&OpaqueStreamRef>,
    ) -> Result<StreamRef<B>, SendError> {
        use super::stream::ContentLength;
        use http::Method;

        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        me.actions.ensure_no_conn_error()?;
        me.actions.send.ensure_next_stream_id()?;

        if let Some(stream) = pending {
            if me.store.resolve(stream.key).is_pending_open {
                return Err(UserError::Rejected.into());
            }
        }

        if me.counts.peer().is_server() {
            return Err(UserError::UnexpectedFrameType.into());
        }

        let stream_id = me.actions.send.open()?;

        let mut stream = Stream::new(
            stream_id,
            me.actions.send.init_window_sz(),
            me.actions.recv.init_window_sz(),
        );

        if *request.method() == Method::HEAD {
            stream.content_length = ContentLength::Head;
        }

        let headers = client::Peer::convert_send_message(stream_id, request, end_of_stream)?;

        let mut stream = me.store.insert(stream.id, stream);

        let sent = me.actions.send.send_headers(
            headers,
            send_buffer,
            &mut stream,
            &mut me.counts,
            &mut me.actions.task,
        );

        if let Err(err) = sent {
            stream.unlink();
            stream.remove();
            return Err(err.into());
        }

        debug_assert!(!stream.state.is_closed());

        me.refs += 1;

        Ok(StreamRef {
            opaque: OpaqueStreamRef::new(self.inner.clone(), &mut stream),
            send_buffer: self.send_buffer.clone(),
        })
    }

    pub fn send_reset(&mut self, id: StreamId, reason: Reason) {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        let key = match me.store.find_entry(id) {
            Entry::Occupied(e) => e.key(),
            Entry::Vacant(e) => {
                let stream = Stream::new(id, 0, 0);

                e.insert(stream)
            }
        };

        let stream = me.store.resolve(key);
        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;
        me.actions
            .send_reset(stream, reason, &mut me.counts, send_buffer);
    }

    pub fn send_go_away(&mut self, last_processed_id: StreamId) {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;
        let actions = &mut me.actions;
        actions.recv.go_away(last_processed_id);
    }
}

impl<B> Streams<B, client::Peer>
where
    B: Buf,
{
    pub fn poll_pending_open(
        &mut self,
        cx: &Context,
        pending: Option<&OpaqueStreamRef>,
    ) -> Poll<Result<(), crate::proto::h2::Error>> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        me.actions.ensure_no_conn_error()?;
        me.actions.send.ensure_next_stream_id()?;

        if let Some(pending) = pending {
            let mut stream = me.store.resolve(pending.key);
            tracing::trace!("poll_pending_open; stream = {:?}", stream.is_pending_open);
            if stream.is_pending_open {
                stream.wait_send(cx);
                return Poll::Pending;
            }
        }
        Poll::Ready(Ok(()))
    }
}

impl<B, P> Streams<B, P>
where
    P: Peer,
{
    pub fn recv_eof(&mut self, clear_pending_accept: bool) -> Result<(), ()> {
        let mut me = self.inner.lock().map_err(|_| ())?;
        let me = &mut *me;

        let actions = &mut me.actions;
        let counts = &mut me.counts;
        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        if actions.conn_error.is_none() {
            actions.conn_error = Some(io::Error::from(io::ErrorKind::BrokenPipe).into());
        }

        tracing::trace!("Streams::recv_eof");

        me.store
            .for_each(|stream| {
                counts.transition(stream, |counts, stream| {
                    actions.recv.recv_eof(stream);

                    actions.send.recv_err(send_buffer, stream, counts);
                    Ok::<_, ()>(())
                })
            })
            .expect("recv_eof");

        actions.clear_queues(clear_pending_accept, &mut me.store, counts);
        Ok(())
    }

    #[cfg(feature = "unstable")]
    pub fn num_active_streams(&self) -> usize {
        let me = self.inner.lock().unwrap();
        me.store.num_active_streams()
    }

    pub fn has_streams(&self) -> bool {
        let me = self.inner.lock().unwrap();
        me.counts.has_streams()
    }

    pub fn has_streams_or_other_references(&self) -> bool {
        let me = self.inner.lock().unwrap();
        me.counts.has_streams() || me.refs > 1
    }

    #[cfg(feature = "unstable")]
    pub fn num_wired_streams(&self) -> usize {
        let me = self.inner.lock().unwrap();
        me.store.num_wired_streams()
    }
}

impl<B, P> Clone for Streams<B, P>
where
    P: Peer,
{
    fn clone(&self) -> Self {
        self.inner.lock().unwrap().refs += 1;
        Streams {
            inner: self.inner.clone(),
            send_buffer: self.send_buffer.clone(),
            _p: ::std::marker::PhantomData,
        }
    }
}

impl<B, P> Drop for Streams<B, P>
where
    P: Peer,
{
    fn drop(&mut self) {
        let _ = self.inner.lock().map(|mut inner| inner.refs -= 1);
    }
}

impl<B> StreamRef<B> {
    pub fn send_data(&mut self, data: B, end_stream: bool) -> Result<(), UserError>
    where
        B: Buf,
    {
        let mut me = self.opaque.inner.lock().unwrap();
        let me = &mut *me;

        let stream = me.store.resolve(self.opaque.key);
        let actions = &mut me.actions;
        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        me.counts.transition(stream, |counts, stream| {
            let mut frame = frame::Data::new(stream.id, data);
            frame.set_end_stream(end_stream);

            actions
                .send
                .send_data(frame, send_buffer, stream, counts, &mut actions.task)
        })
    }

    pub fn send_trailers(&mut self, trailers: HeaderMap) -> Result<(), UserError> {
        let mut me = self.opaque.inner.lock().unwrap();
        let me = &mut *me;

        let stream = me.store.resolve(self.opaque.key);
        let actions = &mut me.actions;
        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        me.counts.transition(stream, |counts, stream| {
            let frame = frame::Headers::trailers(stream.id, trailers);

            actions
                .send
                .send_trailers(frame, send_buffer, stream, counts, &mut actions.task)
        })
    }

    pub fn send_reset(&mut self, reason: Reason) {
        let mut me = self.opaque.inner.lock().unwrap();
        let me = &mut *me;

        let stream = me.store.resolve(self.opaque.key);
        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        me.actions
            .send_reset(stream, reason, &mut me.counts, send_buffer);
    }

    pub fn send_response(
        &mut self,
        response: Response<()>,
        end_of_stream: bool,
    ) -> Result<(), UserError> {
        let mut me = self.opaque.inner.lock().unwrap();
        let me = &mut *me;

        let stream = me.store.resolve(self.opaque.key);
        let actions = &mut me.actions;
        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        me.counts.transition(stream, |counts, stream| {
            let frame = server::Peer::convert_send_message(stream.id, response, end_of_stream);

            actions
                .send
                .send_headers(frame, send_buffer, stream, counts, &mut actions.task)
        })
    }

    pub fn send_push_promise(&mut self, request: Request<()>) -> Result<StreamRef<B>, UserError> {
        let mut me = self.opaque.inner.lock().unwrap();
        let me = &mut *me;

        let mut send_buffer = self.send_buffer.inner.lock().unwrap();
        let send_buffer = &mut *send_buffer;

        let actions = &mut me.actions;
        let promised_id = actions.send.reserve_local()?;

        let child_key = {
            let mut child_stream = me.store.insert(
                promised_id,
                Stream::new(
                    promised_id,
                    actions.send.init_window_sz(),
                    actions.recv.init_window_sz(),
                ),
            );
            child_stream.state.reserve_local()?;
            child_stream.is_pending_push = true;
            child_stream.key()
        };

        let pushed = {
            let mut stream = me.store.resolve(self.opaque.key);

            let frame = crate::proto::h2::server::Peer::convert_push_message(
                stream.id,
                promised_id,
                request,
            )?;

            actions
                .send
                .send_push_promise(frame, send_buffer, &mut stream, &mut actions.task)
        };

        if let Err(err) = pushed {
            let mut child_stream = me.store.resolve(child_key);
            child_stream.unlink();
            child_stream.remove();
            return Err(err.into());
        }

        me.refs += 1;
        let opaque =
            OpaqueStreamRef::new(self.opaque.inner.clone(), &mut me.store.resolve(child_key));

        Ok(StreamRef {
            opaque,
            send_buffer: self.send_buffer.clone(),
        })
    }

    pub fn take_request(&self) -> Request<()> {
        let mut me = self.opaque.inner.lock().unwrap();
        let me = &mut *me;

        let mut stream = me.store.resolve(self.opaque.key);
        me.actions.recv.take_request(&mut stream)
    }

    pub fn is_pending_open(&self) -> bool {
        let mut me = self.opaque.inner.lock().unwrap();
        me.store.resolve(self.opaque.key).is_pending_open
    }

    pub fn reserve_capacity(&mut self, capacity: WindowSize) {
        let mut me = self.opaque.inner.lock().unwrap();
        let me = &mut *me;

        let mut stream = me.store.resolve(self.opaque.key);

        me.actions
            .send
            .reserve_capacity(capacity, &mut stream, &mut me.counts)
    }

    pub fn capacity(&self) -> WindowSize {
        let mut me = self.opaque.inner.lock().unwrap();
        let me = &mut *me;

        let mut stream = me.store.resolve(self.opaque.key);

        me.actions.send.capacity(&mut stream)
    }

    pub fn poll_capacity(&mut self, cx: &Context) -> Poll<Option<Result<WindowSize, UserError>>> {
        let mut me = self.opaque.inner.lock().unwrap();
        let me = &mut *me;

        let mut stream = me.store.resolve(self.opaque.key);

        me.actions.send.poll_capacity(cx, &mut stream)
    }

    pub(crate) fn poll_reset(
        &mut self,
        cx: &Context,
        mode: proto::PollReset,
    ) -> Poll<Result<Reason, crate::proto::h2::Error>> {
        let mut me = self.opaque.inner.lock().unwrap();
        let me = &mut *me;

        let mut stream = me.store.resolve(self.opaque.key);

        me.actions
            .send
            .poll_reset(cx, &mut stream, mode)
            .map_err(From::from)
    }

    pub fn clone_to_opaque(&self) -> OpaqueStreamRef
    where
        B: 'static,
    {
        self.opaque.clone()
    }

    pub fn stream_id(&self) -> StreamId {
        self.opaque.stream_id()
    }
}

impl<B> Clone for StreamRef<B> {
    fn clone(&self) -> Self {
        StreamRef {
            opaque: self.opaque.clone(),
            send_buffer: self.send_buffer.clone(),
        }
    }
}

impl OpaqueStreamRef {
    fn new(inner: Arc<Mutex<Inner>>, stream: &mut store::Ptr) -> OpaqueStreamRef {
        stream.ref_inc();
        OpaqueStreamRef {
            inner,
            key: stream.key(),
        }
    }

    pub fn poll_response(&mut self, cx: &Context) -> Poll<Result<Response<()>, proto::Error>> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        let mut stream = me.store.resolve(self.key);
        me.actions.recv.poll_response(cx, &mut stream)
    }

    pub fn poll_pushed(
        &mut self,
        cx: &Context,
    ) -> Poll<Option<Result<(Request<()>, OpaqueStreamRef), proto::Error>>> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        let mut stream = me.store.resolve(self.key);
        me.actions
            .recv
            .poll_pushed(cx, &mut stream)
            .map_ok_(|(h, key)| {
                me.refs += 1;
                let opaque_ref =
                    OpaqueStreamRef::new(self.inner.clone(), &mut me.store.resolve(key));
                (h, opaque_ref)
            })
    }

    pub fn is_end_stream(&self) -> bool {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        let stream = me.store.resolve(self.key);

        me.actions.recv.is_end_stream(&stream)
    }

    pub fn poll_data(&mut self, cx: &Context) -> Poll<Option<Result<Bytes, proto::Error>>> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        let mut stream = me.store.resolve(self.key);

        me.actions.recv.poll_data(cx, &mut stream)
    }

    pub fn poll_trailers(&mut self, cx: &Context) -> Poll<Option<Result<HeaderMap, proto::Error>>> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        let mut stream = me.store.resolve(self.key);

        me.actions.recv.poll_trailers(cx, &mut stream)
    }

    pub(crate) fn available_recv_capacity(&self) -> isize {
        let me = self.inner.lock().unwrap();
        let me = &*me;

        let stream = &me.store[self.key];
        stream.recv_flow.available().into()
    }

    pub(crate) fn used_recv_capacity(&self) -> WindowSize {
        let me = self.inner.lock().unwrap();
        let me = &*me;

        let stream = &me.store[self.key];
        stream.in_flight_recv_data
    }

    pub fn release_capacity(&mut self, capacity: WindowSize) -> Result<(), UserError> {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        let mut stream = me.store.resolve(self.key);

        me.actions
            .recv
            .release_capacity(capacity, &mut stream, &mut me.actions.task)
    }

    pub(crate) fn clear_recv_buffer(&mut self) {
        let mut me = self.inner.lock().unwrap();
        let me = &mut *me;

        let mut stream = me.store.resolve(self.key);

        me.actions.recv.clear_recv_buffer(&mut stream);
    }

    pub fn stream_id(&self) -> StreamId {
        self.inner.lock().unwrap().store[self.key].id
    }
}

impl fmt::Debug for OpaqueStreamRef {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        use std::sync::TryLockError::*;

        match self.inner.try_lock() {
            Ok(me) => {
                let stream = &me.store[self.key];
                fmt.debug_struct("OpaqueStreamRef")
                    .field("stream_id", &stream.id)
                    .field("ref_count", &stream.ref_count)
                    .finish()
            }
            Err(Poisoned(_)) => fmt
                .debug_struct("OpaqueStreamRef")
                .field("inner", &"<Poisoned>")
                .finish(),
            Err(WouldBlock) => fmt
                .debug_struct("OpaqueStreamRef")
                .field("inner", &"<Locked>")
                .finish(),
        }
    }
}

impl Clone for OpaqueStreamRef {
    fn clone(&self) -> Self {
        let mut inner = self.inner.lock().unwrap();
        inner.store.resolve(self.key).ref_inc();
        inner.refs += 1;

        OpaqueStreamRef {
            inner: self.inner.clone(),
            key: self.key.clone(),
        }
    }
}

impl Drop for OpaqueStreamRef {
    fn drop(&mut self) {
        drop_stream_ref(&self.inner, self.key);
    }
}

fn drop_stream_ref(inner: &Mutex<Inner>, key: store::Key) {
    let mut me = match inner.lock() {
        Ok(inner) => inner,
        Err(_) => {
            if ::std::thread::panicking() {
                tracing::trace!("StreamRef::drop; mutex poisoned");
                return;
            } else {
                panic!("StreamRef::drop; mutex poisoned");
            }
        }
    };

    let me = &mut *me;
    me.refs -= 1;
    let mut stream = me.store.resolve(key);

    tracing::trace!("drop_stream_ref; stream={:?}", stream);

    stream.ref_dec();

    let actions = &mut me.actions;

    if stream.ref_count == 0 && stream.is_closed() {
        if let Some(task) = actions.task.take() {
            task.wake();
        }
    }

    me.counts.transition(stream, |counts, stream| {
        maybe_cancel(stream, actions, counts);

        if stream.ref_count == 0 {
            actions
                .recv
                .release_closed_capacity(stream, &mut actions.task);

            let mut ppp = stream.pending_push_promises.take();
            while let Some(promise) = ppp.pop(stream.store_mut()) {
                counts.transition(promise, |counts, stream| {
                    maybe_cancel(stream, actions, counts);
                });
            }
        }
    });
}

fn maybe_cancel(stream: &mut store::Ptr, actions: &mut Actions, counts: &mut Counts) {
    if stream.is_canceled_interest() {
        actions
            .send
            .schedule_implicit_reset(stream, Reason::CANCEL, counts, &mut actions.task);
        actions.recv.enqueue_reset_expiration(stream, counts);
    }
}

impl<B> SendBuffer<B> {
    fn new() -> Self {
        let inner = Mutex::new(Buffer::new());
        SendBuffer { inner }
    }
}

impl Actions {
    fn send_reset<B>(
        &mut self,
        stream: store::Ptr,
        reason: Reason,
        counts: &mut Counts,
        send_buffer: &mut Buffer<Frame<B>>,
    ) {
        counts.transition(stream, |counts, stream| {
            self.send
                .send_reset(reason, send_buffer, stream, counts, &mut self.task);
            self.recv.enqueue_reset_expiration(stream, counts);
            stream.notify_recv();
        });
    }

    fn reset_on_recv_stream_err<B>(
        &mut self,
        buffer: &mut Buffer<Frame<B>>,
        stream: &mut store::Ptr,
        counts: &mut Counts,
        res: Result<(), RecvError>,
    ) -> Result<(), RecvError> {
        if let Err(RecvError::Stream { reason, .. }) = res {
            self.send
                .send_reset(reason, buffer, stream, counts, &mut self.task);
            Ok(())
        } else {
            res
        }
    }

    fn ensure_not_idle(&mut self, peer: peer::Dyn, id: StreamId) -> Result<(), Reason> {
        if peer.is_local_init(id) {
            self.send.ensure_not_idle(id)
        } else {
            self.recv.ensure_not_idle(id)
        }
    }

    fn ensure_no_conn_error(&self) -> Result<(), proto::Error> {
        if let Some(ref err) = self.conn_error {
            Err(err.shallow_clone())
        } else {
            Ok(())
        }
    }

    fn may_have_forgotten_stream<P: Peer>(&self, id: StreamId) -> bool {
        if id.is_zero() {
            return false;
        }
        if P::is_local_init(id) {
            self.send.may_have_created_stream(id)
        } else {
            self.recv.may_have_created_stream(id)
        }
    }

    fn clear_queues(&mut self, clear_pending_accept: bool, store: &mut Store, counts: &mut Counts) {
        self.recv.clear_queues(clear_pending_accept, store, counts);
        self.send.clear_queues(store, counts);
    }
}
