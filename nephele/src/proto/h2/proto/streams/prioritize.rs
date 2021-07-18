use bytes::buf::{Buf, Take};
use cynthia::future::swap::AsyncWrite;
use std::io;
use std::task::{Context, Poll, Waker};
use std::{cmp, fmt, mem};

use super::*;
use crate::proto::h2::codec::UserError;
use crate::proto::h2::codec::UserError::*;
use crate::proto::h2::frame::{Reason, StreamId};
use crate::proto::h2::proto::streams::store::Resolve;

#[derive(Debug)]
pub(super) struct Prioritize {
    pending_send: store::Queue<stream::NextSend>,
    pending_capacity: store::Queue<stream::NextSendCapacity>,
    pending_open: store::Queue<stream::NextOpen>,
    flow: FlowControl,
    last_opened_id: StreamId,
    in_flight_data_frame: InFlightData,
}

#[derive(Debug, Eq, PartialEq)]
enum InFlightData {
    Nothing,
    DataFrame(store::Key),
    Drop,
}

pub(crate) struct Prioritized<B> {
    inner: Take<B>,
    end_of_stream: bool,
    stream: store::Key,
}

impl Prioritize {
    pub fn new(config: &Config) -> Prioritize {
        let mut flow = FlowControl::new();

        flow.inc_window(config.remote_init_window_sz)
            .expect("invalid initial window size");

        flow.assign_capacity(config.remote_init_window_sz);

        tracing::trace!("Prioritize::new; flow={:?}", flow);

        Prioritize {
            pending_send: store::Queue::new(),
            pending_capacity: store::Queue::new(),
            pending_open: store::Queue::new(),
            flow,
            last_opened_id: StreamId::ZERO,
            in_flight_data_frame: InFlightData::Nothing,
        }
    }

    pub fn queue_frame<B>(
        &mut self,
        frame: Frame<B>,
        buffer: &mut Buffer<Frame<B>>,
        stream: &mut store::Ptr,
        task: &mut Option<Waker>,
    ) {
        let span = tracing::trace_span!("Prioritize::queue_frame", ?stream.id);
        let _e = span.enter();
        stream.pending_send.push_back(buffer, frame);
        self.schedule_send(stream, task);
    }

    pub fn schedule_send(&mut self, stream: &mut store::Ptr, task: &mut Option<Waker>) {
        if stream.is_send_ready() {
            tracing::trace!(?stream.id, "schedule_send");
            self.pending_send.push(stream);

            if let Some(task) = task.take() {
                task.wake();
            }
        }
    }

    pub fn queue_open(&mut self, stream: &mut store::Ptr) {
        self.pending_open.push(stream);
    }

    pub fn send_data<B>(
        &mut self,
        frame: frame::Data<B>,
        buffer: &mut Buffer<Frame<B>>,
        stream: &mut store::Ptr,
        counts: &mut Counts,
        task: &mut Option<Waker>,
    ) -> Result<(), UserError>
    where
        B: Buf,
    {
        let sz = frame.payload().remaining();

        if sz > MAX_WINDOW_SIZE as usize {
            return Err(UserError::PayloadTooBig);
        }

        let sz = sz as WindowSize;

        if !stream.state.is_send_streaming() {
            if stream.state.is_closed() {
                return Err(InactiveStreamId);
            } else {
                return Err(UnexpectedFrameType);
            }
        }

        stream.buffered_send_data += sz;

        let span =
            tracing::trace_span!("send_data", sz, requested = stream.requested_send_capacity);
        let _e = span.enter();
        tracing::trace!(buffered = stream.buffered_send_data);

        if stream.requested_send_capacity < stream.buffered_send_data {
            stream.requested_send_capacity = stream.buffered_send_data;

            self.try_assign_capacity(stream);
        }

        if frame.is_end_stream() {
            stream.state.send_close();
            self.reserve_capacity(0, stream, counts);
        }

        tracing::trace!(
            available = %stream.send_flow.available(),
            buffered = stream.buffered_send_data,
        );

        if stream.send_flow.available() > 0 || stream.buffered_send_data == 0 {
            self.queue_frame(frame.into(), buffer, stream, task);
        } else {
            stream.pending_send.push_back(buffer, frame.into());
        }

        Ok(())
    }

    pub fn reserve_capacity(
        &mut self,
        capacity: WindowSize,
        stream: &mut store::Ptr,
        counts: &mut Counts,
    ) {
        let span = tracing::trace_span!(
            "reserve_capacity",
            ?stream.id,
            requested = capacity,
            effective = capacity + stream.buffered_send_data,
            curr = stream.requested_send_capacity
        );
        let _e = span.enter();

        let capacity = capacity + stream.buffered_send_data;

        if capacity == stream.requested_send_capacity {
        } else if capacity < stream.requested_send_capacity {
            stream.requested_send_capacity = capacity;

            let available = stream.send_flow.available().as_size();

            if available > capacity {
                let diff = available - capacity;

                stream.send_flow.claim_capacity(diff);

                self.assign_connection_capacity(diff, stream, counts);
            }
        } else {
            if stream.state.is_send_closed() {
                return;
            }

            stream.requested_send_capacity = capacity;

            self.try_assign_capacity(stream);
        }
    }

    pub fn recv_stream_window_update(
        &mut self,
        inc: WindowSize,
        stream: &mut store::Ptr,
    ) -> Result<(), Reason> {
        let span = tracing::trace_span!(
            "recv_stream_window_update",
            ?stream.id,
            ?stream.state,
            inc,
            flow = ?stream.send_flow
        );
        let _e = span.enter();

        if stream.state.is_send_closed() && stream.buffered_send_data == 0 {
            return Ok(());
        }

        stream.send_flow.inc_window(inc)?;

        self.try_assign_capacity(stream);

        Ok(())
    }

    pub fn recv_connection_window_update(
        &mut self,
        inc: WindowSize,
        store: &mut Store,
        counts: &mut Counts,
    ) -> Result<(), Reason> {
        self.flow.inc_window(inc)?;

        self.assign_connection_capacity(inc, store, counts);
        Ok(())
    }

    pub fn reclaim_all_capacity(&mut self, stream: &mut store::Ptr, counts: &mut Counts) {
        let available = stream.send_flow.available().as_size();
        stream.send_flow.claim_capacity(available);
        self.assign_connection_capacity(available, stream, counts);
    }

    pub fn reclaim_reserved_capacity(&mut self, stream: &mut store::Ptr, counts: &mut Counts) {
        if stream.requested_send_capacity > stream.buffered_send_data {
            let reserved = stream.requested_send_capacity - stream.buffered_send_data;

            stream.send_flow.claim_capacity(reserved);
            self.assign_connection_capacity(reserved, stream, counts);
        }
    }

    pub fn clear_pending_capacity(&mut self, store: &mut Store, counts: &mut Counts) {
        let span = tracing::trace_span!("clear_pending_capacity");
        let _e = span.enter();
        while let Some(stream) = self.pending_capacity.pop(store) {
            counts.transition(stream, |_, stream| {
                tracing::trace!(?stream.id, "clear_pending_capacity");
            })
        }
    }

    pub fn assign_connection_capacity<R>(
        &mut self,
        inc: WindowSize,
        store: &mut R,
        counts: &mut Counts,
    ) where
        R: Resolve,
    {
        let span = tracing::trace_span!("assign_connection_capacity", inc);
        let _e = span.enter();

        self.flow.assign_capacity(inc);

        while self.flow.available() > 0 {
            let stream = match self.pending_capacity.pop(store) {
                Some(stream) => stream,
                None => return,
            };

            if !(stream.state.is_send_streaming() || stream.buffered_send_data > 0) {
                continue;
            }

            counts.transition(stream, |_, mut stream| {
                self.try_assign_capacity(&mut stream);
            })
        }
    }

    fn try_assign_capacity(&mut self, stream: &mut store::Ptr) {
        let total_requested = stream.requested_send_capacity;

        debug_assert!(total_requested >= stream.send_flow.available());

        let additional = cmp::min(
            total_requested - stream.send_flow.available().as_size(),
            stream.send_flow.window_size() - stream.send_flow.available().as_size(),
        );
        let span = tracing::trace_span!("try_assign_capacity", ?stream.id);
        let _e = span.enter();
        tracing::trace!(
            requested = total_requested,
            additional,
            buffered = stream.buffered_send_data,
            window = stream.send_flow.window_size(),
            conn = %self.flow.available()
        );

        if additional == 0 {
            return;
        }

        debug_assert!(
            stream.state.is_send_streaming() || stream.buffered_send_data > 0,
            "state={:?}",
            stream.state
        );

        let conn_available = self.flow.available().as_size();

        if conn_available > 0 {
            let assign = cmp::min(conn_available, additional);

            tracing::trace!(capacity = assign, "assigning");

            stream.assign_capacity(assign);

            self.flow.claim_capacity(assign);
        }

        tracing::trace!(
            available = %stream.send_flow.available(),
            requested = stream.requested_send_capacity,
            buffered = stream.buffered_send_data,
            has_unavailable = %stream.send_flow.has_unavailable()
        );

        if stream.send_flow.available() < stream.requested_send_capacity
            && stream.send_flow.has_unavailable()
        {
            self.pending_capacity.push(stream);
        }

        if stream.buffered_send_data > 0 && stream.is_send_ready() {
            self.pending_send.push(stream);
        }
    }

    pub fn poll_complete<T, B>(
        &mut self,
        cx: &mut Context,
        buffer: &mut Buffer<Frame<B>>,
        store: &mut Store,
        counts: &mut Counts,
        dst: &mut Codec<T, Prioritized<B>>,
    ) -> Poll<io::Result<()>>
    where
        T: AsyncWrite + Unpin,
        B: Buf,
    {
        ready!(dst.poll_ready(cx))?;

        self.reclaim_frame(buffer, store, dst);

        let max_frame_len = dst.max_send_frame_size();

        tracing::trace!("poll_complete");

        loop {
            self.schedule_pending_open(store, counts);

            match self.pop_frame(buffer, store, max_frame_len, counts) {
                Some(frame) => {
                    tracing::trace!(?frame, "writing");

                    debug_assert_eq!(self.in_flight_data_frame, InFlightData::Nothing);
                    if let Frame::Data(ref frame) = frame {
                        self.in_flight_data_frame = InFlightData::DataFrame(frame.payload().stream);
                    }
                    dst.buffer(frame).expect("invalid frame");

                    ready!(dst.poll_ready(cx))?;

                    self.reclaim_frame(buffer, store, dst);
                }
                None => {
                    ready!(dst.flush(cx))?;

                    if !self.reclaim_frame(buffer, store, dst) {
                        return Poll::Ready(Ok(()));
                    }
                }
            }
        }
    }

    fn reclaim_frame<T, B>(
        &mut self,
        buffer: &mut Buffer<Frame<B>>,
        store: &mut Store,
        dst: &mut Codec<T, Prioritized<B>>,
    ) -> bool
    where
        B: Buf,
    {
        let span = tracing::trace_span!("try_reclaim_frame");
        let _e = span.enter();

        if let Some(frame) = dst.take_last_data_frame() {
            tracing::trace!(
                ?frame,
                sz = frame.payload().inner.get_ref().remaining(),
                "reclaimed"
            );

            let mut eos = false;
            let key = frame.payload().stream;

            match mem::replace(&mut self.in_flight_data_frame, InFlightData::Nothing) {
                InFlightData::Nothing => panic!("wasn't expecting a frame to reclaim"),
                InFlightData::Drop => {
                    tracing::trace!("not reclaiming frame for cancelled stream");
                    return false;
                }
                InFlightData::DataFrame(k) => {
                    debug_assert_eq!(k, key);
                }
            }

            let mut frame = frame.map(|prioritized| {
                eos = prioritized.end_of_stream;
                prioritized.inner.into_inner()
            });

            if frame.payload().has_remaining() {
                let mut stream = store.resolve(key);

                if eos {
                    frame.set_end_stream(true);
                }

                self.push_back_frame(frame.into(), buffer, &mut stream);

                return true;
            }
        }

        false
    }

    fn push_back_frame<B>(
        &mut self,
        frame: Frame<B>,
        buffer: &mut Buffer<Frame<B>>,
        stream: &mut store::Ptr,
    ) {
        stream.pending_send.push_front(buffer, frame);

        if stream.send_flow.available() > 0 {
            debug_assert!(!stream.pending_send.is_empty());
            self.pending_send.push(stream);
        }
    }

    pub fn clear_queue<B>(&mut self, buffer: &mut Buffer<Frame<B>>, stream: &mut store::Ptr) {
        let span = tracing::trace_span!("clear_queue", ?stream.id);
        let _e = span.enter();

        while let Some(frame) = stream.pending_send.pop_front(buffer) {
            tracing::trace!(?frame, "dropping");
        }

        stream.buffered_send_data = 0;
        stream.requested_send_capacity = 0;
        if let InFlightData::DataFrame(key) = self.in_flight_data_frame {
            if stream.key() == key {
                self.in_flight_data_frame = InFlightData::Drop;
            }
        }
    }

    pub fn clear_pending_send(&mut self, store: &mut Store, counts: &mut Counts) {
        while let Some(stream) = self.pending_send.pop(store) {
            let is_pending_reset = stream.is_pending_reset_expiration();
            counts.transition_after(stream, is_pending_reset);
        }
    }

    pub fn clear_pending_open(&mut self, store: &mut Store, counts: &mut Counts) {
        while let Some(stream) = self.pending_open.pop(store) {
            let is_pending_reset = stream.is_pending_reset_expiration();
            counts.transition_after(stream, is_pending_reset);
        }
    }

    fn pop_frame<B>(
        &mut self,
        buffer: &mut Buffer<Frame<B>>,
        store: &mut Store,
        max_len: usize,
        counts: &mut Counts,
    ) -> Option<Frame<Prioritized<B>>>
    where
        B: Buf,
    {
        let span = tracing::trace_span!("pop_frame");
        let _e = span.enter();

        loop {
            match self.pending_send.pop(store) {
                Some(mut stream) => {
                    let span = tracing::trace_span!("popped", ?stream.id, ?stream.state);
                    let _e = span.enter();

                    let is_pending_reset = stream.is_pending_reset_expiration();

                    tracing::trace!(is_pending_reset);

                    let frame = match stream.pending_send.pop_front(buffer) {
                        Some(Frame::Data(mut frame)) => {
                            let stream_capacity = stream.send_flow.available();
                            let sz = frame.payload().remaining();

                            tracing::trace!(
                                sz,
                                eos = frame.is_end_stream(),
                                window = %stream_capacity,
                                available = %stream.send_flow.available(),
                                requested = stream.requested_send_capacity,
                                buffered = stream.buffered_send_data,
                                "data frame"
                            );

                            if sz > 0 && stream_capacity == 0 {
                                tracing::trace!("stream capacity is 0");

                                stream.pending_send.push_front(buffer, frame.into());

                                continue;
                            }

                            let len = cmp::min(sz, max_len);

                            let len =
                                cmp::min(len, stream_capacity.as_size() as usize) as WindowSize;

                            debug_assert!(len <= self.flow.window_size());

                            tracing::trace!(len, "sending data frame");

                            tracing::trace_span!("updating stream flow").in_scope(|| {
                                stream.send_flow.send_data(len);

                                debug_assert!(stream.buffered_send_data >= len);
                                stream.buffered_send_data -= len;
                                stream.requested_send_capacity -= len;

                                self.flow.assign_capacity(len);
                            });

                            let (eos, len) = tracing::trace_span!("updating connection flow")
                                .in_scope(|| {
                                    self.flow.send_data(len);
                                    let eos = frame.is_end_stream();
                                    let len = len as usize;

                                    if frame.payload().remaining() > len {
                                        frame.set_end_stream(false);
                                    }
                                    (eos, len)
                                });

                            Frame::Data(frame.map(|buf| Prioritized {
                                inner: buf.take(len),
                                end_of_stream: eos,
                                stream: stream.key(),
                            }))
                        }
                        Some(Frame::PushPromise(pp)) => {
                            let mut pushed =
                                stream.store_mut().find_mut(&pp.promised_id()).unwrap();
                            pushed.is_pending_push = false;
                            if !pushed.pending_send.is_empty() {
                                if counts.can_inc_num_send_streams() {
                                    counts.inc_num_send_streams(&mut pushed);
                                    self.pending_send.push(&mut pushed);
                                } else {
                                    self.queue_open(&mut pushed);
                                }
                            }
                            Frame::PushPromise(pp)
                        }
                        Some(frame) => frame.map(|_| {
                            unreachable!(
                                "Frame::map closure will only be called \
                                 on DATA frames."
                            )
                        }),
                        None => {
                            if let Some(reason) = stream.state.get_scheduled_reset() {
                                stream.state.set_reset(reason);

                                let frame = frame::Reset::new(stream.id, reason);
                                Frame::Reset(frame)
                            } else {
                                tracing::trace!("removing dangling stream from pending_send");
                                debug_assert!(stream.state.is_closed());
                                counts.transition_after(stream, is_pending_reset);
                                continue;
                            }
                        }
                    };

                    tracing::trace!("pop_frame; frame={:?}", frame);

                    if cfg!(debug_assertions) && stream.state.is_idle() {
                        debug_assert!(stream.id > self.last_opened_id);
                        self.last_opened_id = stream.id;
                    }

                    if !stream.pending_send.is_empty() || stream.state.is_scheduled_reset() {
                        self.pending_send.push(&mut stream);
                    }

                    counts.transition_after(stream, is_pending_reset);

                    return Some(frame);
                }
                None => return None,
            }
        }
    }

    fn schedule_pending_open(&mut self, store: &mut Store, counts: &mut Counts) {
        tracing::trace!("schedule_pending_open");
        while counts.can_inc_num_send_streams() {
            if let Some(mut stream) = self.pending_open.pop(store) {
                tracing::trace!("schedule_pending_open; stream={:?}", stream.id);

                counts.inc_num_send_streams(&mut stream);
                self.pending_send.push(&mut stream);
                stream.notify_send();
            } else {
                return;
            }
        }
    }
}

impl<B> Buf for Prioritized<B>
where
    B: Buf,
{
    fn remaining(&self) -> usize {
        self.inner.remaining()
    }

    fn chunk(&self) -> &[u8] {
        self.inner.chunk()
    }

    fn chunks_vectored<'a>(&'a self, dst: &mut [std::io::IoSlice<'a>]) -> usize {
        self.inner.chunks_vectored(dst)
    }

    fn advance(&mut self, cnt: usize) {
        self.inner.advance(cnt)
    }
}

impl<B: Buf> fmt::Debug for Prioritized<B> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("Prioritized")
            .field("remaining", &self.inner.get_ref().remaining())
            .field("end_of_stream", &self.end_of_stream)
            .field("stream", &self.stream)
            .finish()
    }
}
