use cynthia::future::swap::AsyncWrite;
use http::{HeaderMap, Request, Response};
use std::io;
use std::task::Context;
use std::task::{Poll, Waker};
use std::time::{Duration, Instant};

use super::*;
use crate::proto::h2::codec::{RecvError, UserError};
use crate::proto::h2::frame::{PushPromiseHeaderError, Reason, DEFAULT_INITIAL_WINDOW_SIZE};
use crate::proto::h2::{frame, proto};

#[derive(Debug)]
pub(super) struct Recv {
    init_window_sz: WindowSize,

    flow: FlowControl,

    in_flight_data: WindowSize,

    next_stream_id: Result<StreamId, StreamIdOverflow>,

    last_processed_id: StreamId,

    max_stream_id: StreamId,

    pending_window_updates: store::Queue<stream::NextWindowUpdate>,

    pending_accept: store::Queue<stream::NextAccept>,

    pending_reset_expired: store::Queue<stream::NextResetExpire>,

    reset_duration: Duration,

    buffer: Buffer<Event>,

    refused: Option<StreamId>,

    is_push_enabled: bool,
}

#[derive(Debug)]
pub(super) enum Event {
    Headers(peer::PollMessage),
    Data(Bytes),
    Trailers(HeaderMap),
}

#[derive(Debug)]
pub(super) enum RecvHeaderBlockError<T> {
    Oversize(T),
    State(RecvError),
}

#[derive(Debug)]
pub(crate) enum Open {
    PushPromise,
    Headers,
}

#[derive(Debug, Clone, Copy)]
struct Indices {
    head: store::Key,
    tail: store::Key,
}

impl Recv {
    pub fn new(peer: peer::Dyn, config: &Config) -> Self {
        let next_stream_id = if peer.is_server() { 1 } else { 2 };

        let mut flow = FlowControl::new();

        flow.inc_window(DEFAULT_INITIAL_WINDOW_SIZE)
            .expect("invalid initial remote window size");
        flow.assign_capacity(DEFAULT_INITIAL_WINDOW_SIZE);

        Recv {
            init_window_sz: config.local_init_window_sz,
            flow,
            in_flight_data: 0 as WindowSize,
            next_stream_id: Ok(next_stream_id.into()),
            pending_window_updates: store::Queue::new(),
            last_processed_id: StreamId::ZERO,
            max_stream_id: StreamId::MAX,
            pending_accept: store::Queue::new(),
            pending_reset_expired: store::Queue::new(),
            reset_duration: config.local_reset_duration,
            buffer: Buffer::new(),
            refused: None,
            is_push_enabled: config.local_push_enabled,
        }
    }

    pub fn init_window_sz(&self) -> WindowSize {
        self.init_window_sz
    }

    pub fn last_processed_id(&self) -> StreamId {
        self.last_processed_id
    }

    pub fn open(
        &mut self,
        id: StreamId,
        mode: Open,
        counts: &mut Counts,
    ) -> Result<Option<StreamId>, RecvError> {
        assert!(self.refused.is_none());

        counts.peer().ensure_can_open(id, mode)?;

        let next_id = self.next_stream_id()?;
        if id < next_id {
            proto_err!(conn: "id ({:?}) < next_id ({:?})", id, next_id);
            return Err(RecvError::Connection(Reason::PROTOCOL_ERROR));
        }

        self.next_stream_id = id.next_id();

        if !counts.can_inc_num_recv_streams() {
            self.refused = Some(id);
            return Ok(None);
        }

        Ok(Some(id))
    }

    pub fn recv_headers(
        &mut self,
        frame: frame::Headers,
        stream: &mut store::Ptr,
        counts: &mut Counts,
    ) -> Result<(), RecvHeaderBlockError<Option<frame::Headers>>> {
        tracing::trace!("opening stream; init_window={}", self.init_window_sz);
        let is_initial = stream.state.recv_open(frame.is_end_stream())?;

        if is_initial {
            if frame.stream_id() > self.last_processed_id {
                self.last_processed_id = frame.stream_id();
            }

            counts.inc_num_recv_streams(stream);
        }

        if !stream.content_length.is_head() {
            use super::stream::ContentLength;
            use http::header;

            if let Some(content_length) = frame.fields().get(header::CONTENT_LENGTH) {
                let content_length = match frame::parse_u64(content_length.as_bytes()) {
                    Ok(v) => v,
                    Err(()) => {
                        proto_err!(stream: "could not parse content-length; stream={:?}", stream.id);
                        return Err(RecvError::Stream {
                            id: stream.id,
                            reason: Reason::PROTOCOL_ERROR,
                        }
                        .into());
                    }
                };

                stream.content_length = ContentLength::Remaining(content_length);
            }
        }

        if frame.is_over_size() {
            tracing::debug!(
                "stream error REQUEST_HEADER_FIELDS_TOO_LARGE -- \
                 recv_headers: frame is over size; stream={:?}",
                stream.id
            );
            return if counts.peer().is_server() && is_initial {
                let mut res = frame::Headers::new(
                    stream.id,
                    frame::Pseudo::response(::http::StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE),
                    HeaderMap::new(),
                );
                res.set_end_stream();
                Err(RecvHeaderBlockError::Oversize(Some(res)))
            } else {
                Err(RecvHeaderBlockError::Oversize(None))
            };
        }

        let stream_id = frame.stream_id();
        let (pseudo, fields) = frame.into_parts();
        let message = counts
            .peer()
            .convert_poll_message(pseudo, fields, stream_id)?;

        stream
            .pending_recv
            .push_back(&mut self.buffer, Event::Headers(message));
        stream.notify_recv();

        if counts.peer().is_server() {
            self.pending_accept.push(stream);
        }

        Ok(())
    }

    pub fn take_request(&mut self, stream: &mut store::Ptr) -> Request<()> {
        use super::peer::PollMessage::*;

        match stream.pending_recv.pop_front(&mut self.buffer) {
            Some(Event::Headers(Server(request))) => request,
            _ => panic!(),
        }
    }

    pub fn poll_pushed(
        &mut self,
        cx: &Context,
        stream: &mut store::Ptr,
    ) -> Poll<Option<Result<(Request<()>, store::Key), proto::Error>>> {
        use super::peer::PollMessage::*;

        let mut ppp = stream.pending_push_promises.take();
        let pushed = ppp.pop(stream.store_mut()).map(|mut pushed| {
            match pushed.pending_recv.pop_front(&mut self.buffer) {
                Some(Event::Headers(Server(headers))) => (headers, pushed.key()),
                _ => panic!("Headers not set on pushed stream"),
            }
        });
        stream.pending_push_promises = ppp;
        if let Some(p) = pushed {
            Poll::Ready(Some(Ok(p)))
        } else {
            let is_open = stream.state.ensure_recv_open()?;

            if is_open {
                stream.recv_task = Some(cx.waker().clone());
                Poll::Pending
            } else {
                Poll::Ready(None)
            }
        }
    }

    pub fn poll_response(
        &mut self,
        cx: &Context,
        stream: &mut store::Ptr,
    ) -> Poll<Result<Response<()>, proto::Error>> {
        use super::peer::PollMessage::*;

        match stream.pending_recv.pop_front(&mut self.buffer) {
            Some(Event::Headers(Client(response))) => Poll::Ready(Ok(response)),
            Some(_) => panic!("poll_response called after response returned"),
            None => {
                stream.state.ensure_recv_open()?;
                stream.recv_task = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }

    pub fn recv_trailers(
        &mut self,
        frame: frame::Headers,
        stream: &mut store::Ptr,
    ) -> Result<(), RecvError> {
        stream.state.recv_close()?;

        if stream.ensure_content_length_zero().is_err() {
            proto_err!(stream: "recv_trailers: content-length is not zero; stream={:?};",  stream.id);
            return Err(RecvError::Stream {
                id: stream.id,
                reason: Reason::PROTOCOL_ERROR,
            });
        }

        let trailers = frame.into_fields();

        stream
            .pending_recv
            .push_back(&mut self.buffer, Event::Trailers(trailers));
        stream.notify_recv();

        Ok(())
    }

    pub fn release_connection_capacity(&mut self, capacity: WindowSize, task: &mut Option<Waker>) {
        tracing::trace!(
            "release_connection_capacity; size={}, connection in_flight_data={}",
            capacity,
            self.in_flight_data,
        );

        self.in_flight_data -= capacity;

        self.flow.assign_capacity(capacity);

        if self.flow.unclaimed_capacity().is_some() {
            if let Some(task) = task.take() {
                task.wake();
            }
        }
    }

    pub fn release_capacity(
        &mut self,
        capacity: WindowSize,
        stream: &mut store::Ptr,
        task: &mut Option<Waker>,
    ) -> Result<(), UserError> {
        tracing::trace!("release_capacity; size={}", capacity);

        if capacity > stream.in_flight_recv_data {
            return Err(UserError::ReleaseCapacityTooBig);
        }

        self.release_connection_capacity(capacity, task);

        stream.in_flight_recv_data -= capacity;

        stream.recv_flow.assign_capacity(capacity);

        if stream.recv_flow.unclaimed_capacity().is_some() {
            self.pending_window_updates.push(stream);

            if let Some(task) = task.take() {
                task.wake();
            }
        }

        Ok(())
    }

    pub fn release_closed_capacity(&mut self, stream: &mut store::Ptr, task: &mut Option<Waker>) {
        debug_assert_eq!(stream.ref_count, 0);

        if stream.in_flight_recv_data == 0 {
            return;
        }

        tracing::trace!(
            "auto-release closed stream ({:?}) capacity: {:?}",
            stream.id,
            stream.in_flight_recv_data,
        );

        self.release_connection_capacity(stream.in_flight_recv_data, task);
        stream.in_flight_recv_data = 0;

        self.clear_recv_buffer(stream);
    }

    pub fn set_target_connection_window(&mut self, target: WindowSize, task: &mut Option<Waker>) {
        tracing::trace!(
            "set_target_connection_window; target={}; available={}, reserved={}",
            target,
            self.flow.available(),
            self.in_flight_data,
        );

        let current = (self.flow.available() + self.in_flight_data).checked_size();
        if target > current {
            self.flow.assign_capacity(target - current);
        } else {
            self.flow.claim_capacity(current - target);
        }

        if self.flow.unclaimed_capacity().is_some() {
            if let Some(task) = task.take() {
                task.wake();
            }
        }
    }

    pub(crate) fn apply_local_settings(
        &mut self,
        settings: &frame::Settings,
        store: &mut Store,
    ) -> Result<(), RecvError> {
        let target = if let Some(val) = settings.initial_window_size() {
            val
        } else {
            return Ok(());
        };

        let old_sz = self.init_window_sz;
        self.init_window_sz = target;

        tracing::trace!("update_initial_window_size; new={}; old={}", target, old_sz,);

        if target < old_sz {
            let dec = old_sz - target;
            tracing::trace!("decrementing all windows; dec={}", dec);

            store.for_each(|mut stream| {
                stream.recv_flow.dec_recv_window(dec);
                Ok(())
            })
        } else if target > old_sz {
            let inc = target - old_sz;
            tracing::trace!("incrementing all windows; inc={}", inc);
            store.for_each(|mut stream| {
                stream
                    .recv_flow
                    .inc_window(inc)
                    .map_err(RecvError::Connection)?;
                stream.recv_flow.assign_capacity(inc);
                Ok(())
            })
        } else {
            Ok(())
        }
    }

    pub fn is_end_stream(&self, stream: &store::Ptr) -> bool {
        if !stream.state.is_recv_closed() {
            return false;
        }

        stream.pending_recv.is_empty()
    }

    pub fn recv_data(
        &mut self,
        frame: frame::Data,
        stream: &mut store::Ptr,
    ) -> Result<(), RecvError> {
        let sz = frame.payload().len();

        assert!(sz <= MAX_WINDOW_SIZE as usize);

        let sz = sz as WindowSize;

        let is_ignoring_frame = stream.state.is_local_reset();

        if !is_ignoring_frame && !stream.state.is_recv_streaming() {
            proto_err!(conn: "unexpected DATA frame; stream={:?}", stream.id);
            return Err(RecvError::Connection(Reason::PROTOCOL_ERROR));
        }

        tracing::trace!(
            "recv_data; size={}; connection={}; stream={}",
            sz,
            self.flow.window_size(),
            stream.recv_flow.window_size()
        );

        if is_ignoring_frame {
            tracing::trace!(
                "recv_data; frame ignored on locally reset {:?} for some time",
                stream.id,
            );
            return self.ignore_data(sz);
        }

        self.consume_connection_window(sz)?;

        if stream.recv_flow.window_size() < sz {
            return Err(RecvError::Stream {
                id: stream.id,
                reason: Reason::FLOW_CONTROL_ERROR,
            });
        }

        if stream.dec_content_length(frame.payload().len()).is_err() {
            proto_err!(stream:
                "recv_data: content-length overflow; stream={:?}; len={:?}",
                stream.id,
                frame.payload().len(),
            );
            return Err(RecvError::Stream {
                id: stream.id,
                reason: Reason::PROTOCOL_ERROR,
            });
        }

        if frame.is_end_stream() {
            if stream.ensure_content_length_zero().is_err() {
                proto_err!(stream:
                    "recv_data: content-length underflow; stream={:?}; len={:?}",
                    stream.id,
                    frame.payload().len(),
                );
                return Err(RecvError::Stream {
                    id: stream.id,
                    reason: Reason::PROTOCOL_ERROR,
                });
            }

            if stream.state.recv_close().is_err() {
                proto_err!(conn: "recv_data: failed to transition to closed state; stream={:?}", stream.id);
                return Err(RecvError::Connection(Reason::PROTOCOL_ERROR));
            }
        }

        stream.recv_flow.send_data(sz);

        stream.in_flight_recv_data += sz;

        let event = Event::Data(frame.into_payload());

        stream.pending_recv.push_back(&mut self.buffer, event);
        stream.notify_recv();

        Ok(())
    }

    pub fn ignore_data(&mut self, sz: WindowSize) -> Result<(), RecvError> {
        self.consume_connection_window(sz)?;

        self.release_connection_capacity(sz, &mut None);
        Ok(())
    }

    pub fn consume_connection_window(&mut self, sz: WindowSize) -> Result<(), RecvError> {
        if self.flow.window_size() < sz {
            tracing::debug!(
                "connection error FLOW_CONTROL_ERROR -- window_size ({:?}) < sz ({:?});",
                self.flow.window_size(),
                sz,
            );
            return Err(RecvError::Connection(Reason::FLOW_CONTROL_ERROR));
        }

        self.flow.send_data(sz);

        self.in_flight_data += sz;
        Ok(())
    }

    pub fn recv_push_promise(
        &mut self,
        frame: frame::PushPromise,
        stream: &mut store::Ptr,
    ) -> Result<(), RecvError> {
        stream.state.reserve_remote()?;
        if frame.is_over_size() {
            tracing::debug!(
                "stream error REFUSED_STREAM -- recv_push_promise: \
                 headers frame is over size; promised_id={:?};",
                frame.promised_id(),
            );
            return Err(RecvError::Stream {
                id: frame.promised_id(),
                reason: Reason::REFUSED_STREAM,
            });
        }

        let promised_id = frame.promised_id();
        let (pseudo, fields) = frame.into_parts();
        let req =
            crate::proto::h2::server::Peer::convert_poll_message(pseudo, fields, promised_id)?;

        if let Err(e) = frame::PushPromise::validate_request(&req) {
            use PushPromiseHeaderError::*;
            match e {
                NotSafeAndCacheable => proto_err!(
                    stream:
                    "recv_push_promise: method {} is not safe and cacheable; promised_id={:?}",
                    req.method(),
                    promised_id,
                ),
                InvalidContentLength(e) => proto_err!(
                    stream:
                    "recv_push_promise; promised request has invalid content-length {:?}; promised_id={:?}",
                    e,
                    promised_id,
                ),
            }
            return Err(RecvError::Stream {
                id: promised_id,
                reason: Reason::PROTOCOL_ERROR,
            });
        }

        use super::peer::PollMessage::*;
        stream
            .pending_recv
            .push_back(&mut self.buffer, Event::Headers(Server(req)));
        stream.notify_recv();
        Ok(())
    }

    pub fn ensure_not_idle(&self, id: StreamId) -> Result<(), Reason> {
        if let Ok(next) = self.next_stream_id {
            if id >= next {
                tracing::debug!(
                    "stream ID implicitly closed, PROTOCOL_ERROR; stream={:?}",
                    id
                );
                return Err(Reason::PROTOCOL_ERROR);
            }
        }

        Ok(())
    }

    pub fn recv_reset(&mut self, frame: frame::Reset, stream: &mut Stream) {
        stream
            .state
            .recv_reset(frame.reason(), stream.is_pending_send);

        stream.notify_send();
        stream.notify_recv();
    }

    pub fn recv_err(&mut self, err: &proto::Error, stream: &mut Stream) {
        stream.state.recv_err(err);

        stream.notify_send();
        stream.notify_recv();
    }

    pub fn go_away(&mut self, last_processed_id: StreamId) {
        assert!(self.max_stream_id >= last_processed_id);
        self.max_stream_id = last_processed_id;
    }

    pub fn recv_eof(&mut self, stream: &mut Stream) {
        stream.state.recv_eof();
        stream.notify_send();
        stream.notify_recv();
    }

    pub(super) fn clear_recv_buffer(&mut self, stream: &mut Stream) {
        while let Some(_) = stream.pending_recv.pop_front(&mut self.buffer) {
            // drop
        }
    }

    pub fn max_stream_id(&self) -> StreamId {
        self.max_stream_id
    }

    pub fn next_stream_id(&self) -> Result<StreamId, RecvError> {
        if let Ok(id) = self.next_stream_id {
            Ok(id)
        } else {
            Err(RecvError::Connection(Reason::PROTOCOL_ERROR))
        }
    }

    pub fn may_have_created_stream(&self, id: StreamId) -> bool {
        if let Ok(next_id) = self.next_stream_id {
            debug_assert_eq!(id.is_server_initiated(), next_id.is_server_initiated(),);
            id < next_id
        } else {
            true
        }
    }

    pub fn ensure_can_reserve(&self) -> Result<(), RecvError> {
        if !self.is_push_enabled {
            proto_err!(conn: "recv_push_promise: push is disabled");
            return Err(RecvError::Connection(Reason::PROTOCOL_ERROR));
        }

        Ok(())
    }

    pub fn enqueue_reset_expiration(&mut self, stream: &mut store::Ptr, counts: &mut Counts) {
        if !stream.state.is_local_reset() || stream.is_pending_reset_expiration() {
            return;
        }

        tracing::trace!("enqueue_reset_expiration; {:?}", stream.id);

        if !counts.can_inc_num_reset_streams() {
            if let Some(evicted) = self.pending_reset_expired.pop(stream.store_mut()) {
                counts.transition_after(evicted, true);
            }
        }

        if counts.can_inc_num_reset_streams() {
            counts.inc_num_reset_streams();
            self.pending_reset_expired.push(stream);
        }
    }

    pub fn send_pending_refusal<T, B>(
        &mut self,
        cx: &mut Context,
        dst: &mut Codec<T, Prioritized<B>>,
    ) -> Poll<io::Result<()>>
    where
        T: AsyncWrite + Unpin,
        B: Buf,
    {
        if let Some(stream_id) = self.refused {
            ready!(dst.poll_ready(cx))?;

            let frame = frame::Reset::new(stream_id, Reason::REFUSED_STREAM);

            dst.buffer(frame.into()).expect("invalid RST_STREAM frame");
        }

        self.refused = None;

        Poll::Ready(Ok(()))
    }

    pub fn clear_expired_reset_streams(&mut self, store: &mut Store, counts: &mut Counts) {
        let now = Instant::now();
        let reset_duration = self.reset_duration;
        while let Some(stream) = self.pending_reset_expired.pop_if(store, |stream| {
            let reset_at = stream.reset_at.expect("reset_at must be set if in queue");
            now - reset_at > reset_duration
        }) {
            counts.transition_after(stream, true);
        }
    }

    pub fn clear_queues(
        &mut self,
        clear_pending_accept: bool,
        store: &mut Store,
        counts: &mut Counts,
    ) {
        self.clear_stream_window_update_queue(store, counts);
        self.clear_all_reset_streams(store, counts);

        if clear_pending_accept {
            self.clear_all_pending_accept(store, counts);
        }
    }

    fn clear_stream_window_update_queue(&mut self, store: &mut Store, counts: &mut Counts) {
        while let Some(stream) = self.pending_window_updates.pop(store) {
            counts.transition(stream, |_, stream| {
                tracing::trace!("clear_stream_window_update_queue; stream={:?}", stream.id);
            })
        }
    }

    fn clear_all_reset_streams(&mut self, store: &mut Store, counts: &mut Counts) {
        while let Some(stream) = self.pending_reset_expired.pop(store) {
            counts.transition_after(stream, true);
        }
    }

    fn clear_all_pending_accept(&mut self, store: &mut Store, counts: &mut Counts) {
        while let Some(stream) = self.pending_accept.pop(store) {
            counts.transition_after(stream, false);
        }
    }

    pub fn poll_complete<T, B>(
        &mut self,
        cx: &mut Context,
        store: &mut Store,
        counts: &mut Counts,
        dst: &mut Codec<T, Prioritized<B>>,
    ) -> Poll<io::Result<()>>
    where
        T: AsyncWrite + Unpin,
        B: Buf,
    {
        ready!(self.send_connection_window_update(cx, dst))?;

        ready!(self.send_stream_window_updates(cx, store, counts, dst))?;

        Poll::Ready(Ok(()))
    }

    fn send_connection_window_update<T, B>(
        &mut self,
        cx: &mut Context,
        dst: &mut Codec<T, Prioritized<B>>,
    ) -> Poll<io::Result<()>>
    where
        T: AsyncWrite + Unpin,
        B: Buf,
    {
        if let Some(incr) = self.flow.unclaimed_capacity() {
            let frame = frame::WindowUpdate::new(StreamId::zero(), incr);

            ready!(dst.poll_ready(cx))?;

            dst.buffer(frame.into())
                .expect("invalid WINDOW_UPDATE frame");

            self.flow
                .inc_window(incr)
                .expect("unexpected flow control state");
        }

        Poll::Ready(Ok(()))
    }

    pub fn send_stream_window_updates<T, B>(
        &mut self,
        cx: &mut Context,
        store: &mut Store,
        counts: &mut Counts,
        dst: &mut Codec<T, Prioritized<B>>,
    ) -> Poll<io::Result<()>>
    where
        T: AsyncWrite + Unpin,
        B: Buf,
    {
        loop {
            ready!(dst.poll_ready(cx))?;

            let stream = match self.pending_window_updates.pop(store) {
                Some(stream) => stream,
                None => return Poll::Ready(Ok(())),
            };

            counts.transition(stream, |_, stream| {
                tracing::trace!("pending_window_updates -- pop; stream={:?}", stream.id);
                debug_assert!(!stream.is_pending_window_update);

                if !stream.state.is_recv_streaming() {
                    return;
                }

                if let Some(incr) = stream.recv_flow.unclaimed_capacity() {
                    let frame = frame::WindowUpdate::new(stream.id, incr);

                    dst.buffer(frame.into())
                        .expect("invalid WINDOW_UPDATE frame");

                    stream
                        .recv_flow
                        .inc_window(incr)
                        .expect("unexpected flow control state");
                }
            })
        }
    }

    pub fn next_incoming(&mut self, store: &mut Store) -> Option<store::Key> {
        self.pending_accept.pop(store).map(|ptr| ptr.key())
    }

    pub fn poll_data(
        &mut self,
        cx: &Context,
        stream: &mut Stream,
    ) -> Poll<Option<Result<Bytes, proto::Error>>> {
        match stream.pending_recv.pop_front(&mut self.buffer) {
            Some(Event::Data(payload)) => Poll::Ready(Some(Ok(payload))),
            Some(event) => {
                stream.pending_recv.push_front(&mut self.buffer, event);

                stream.notify_recv();

                Poll::Ready(None)
            }
            None => self.schedule_recv(cx, stream),
        }
    }

    pub fn poll_trailers(
        &mut self,
        cx: &Context,
        stream: &mut Stream,
    ) -> Poll<Option<Result<HeaderMap, proto::Error>>> {
        match stream.pending_recv.pop_front(&mut self.buffer) {
            Some(Event::Trailers(trailers)) => Poll::Ready(Some(Ok(trailers))),
            Some(event) => {
                stream.pending_recv.push_front(&mut self.buffer, event);

                Poll::Pending
            }
            None => self.schedule_recv(cx, stream),
        }
    }

    fn schedule_recv<T>(
        &mut self,
        cx: &Context,
        stream: &mut Stream,
    ) -> Poll<Option<Result<T, proto::Error>>> {
        if stream.state.ensure_recv_open()? {
            stream.recv_task = Some(cx.waker().clone());
            Poll::Pending
        } else {
            Poll::Ready(None)
        }
    }
}

impl Open {
    pub fn is_push_promise(&self) -> bool {
        use self::Open::*;

        match *self {
            PushPromise => true,
            _ => false,
        }
    }
}

impl<T> From<RecvError> for RecvHeaderBlockError<T> {
    fn from(err: RecvError) -> Self {
        RecvHeaderBlockError::State(err)
    }
}
