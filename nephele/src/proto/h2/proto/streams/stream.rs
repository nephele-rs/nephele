use std::task::{Context, Waker};
use std::time::Instant;
use std::usize;

use super::*;

#[derive(Debug)]
pub(super) struct Stream {
    pub id: StreamId,
    pub state: State,
    pub is_counted: bool,
    pub ref_count: usize,
    pub next_pending_send: Option<store::Key>,
    pub is_pending_send: bool,
    pub send_flow: FlowControl,
    pub requested_send_capacity: WindowSize,
    pub buffered_send_data: WindowSize,
    send_task: Option<Waker>,
    pub pending_send: buffer::Deque,
    pub next_pending_send_capacity: Option<store::Key>,
    pub is_pending_send_capacity: bool,
    pub send_capacity_inc: bool,
    pub next_open: Option<store::Key>,
    pub is_pending_open: bool,
    pub is_pending_push: bool,
    pub next_pending_accept: Option<store::Key>,
    pub is_pending_accept: bool,
    pub recv_flow: FlowControl,
    pub in_flight_recv_data: WindowSize,
    pub next_window_update: Option<store::Key>,
    pub is_pending_window_update: bool,
    pub reset_at: Option<Instant>,
    pub next_reset_expire: Option<store::Key>,
    pub pending_recv: buffer::Deque,
    pub recv_task: Option<Waker>,
    pub pending_push_promises: store::Queue<NextAccept>,
    pub content_length: ContentLength,
}

#[derive(Debug)]
pub enum ContentLength {
    Omitted,
    Head,
    Remaining(u64),
}

#[derive(Debug)]
pub(super) struct NextAccept;

#[derive(Debug)]
pub(super) struct NextSend;

#[derive(Debug)]
pub(super) struct NextSendCapacity;

#[derive(Debug)]
pub(super) struct NextWindowUpdate;

#[derive(Debug)]
pub(super) struct NextOpen;

#[derive(Debug)]
pub(super) struct NextResetExpire;

impl Stream {
    pub fn new(id: StreamId, init_send_window: WindowSize, init_recv_window: WindowSize) -> Stream {
        let mut send_flow = FlowControl::new();
        let mut recv_flow = FlowControl::new();

        recv_flow
            .inc_window(init_recv_window)
            .expect("invalid initial receive window");
        recv_flow.assign_capacity(init_recv_window);

        send_flow
            .inc_window(init_send_window)
            .expect("invalid initial send window size");

        Stream {
            id,
            state: State::default(),
            ref_count: 0,
            is_counted: false,

            next_pending_send: None,
            is_pending_send: false,
            send_flow,
            requested_send_capacity: 0,
            buffered_send_data: 0,
            send_task: None,
            pending_send: buffer::Deque::new(),
            is_pending_send_capacity: false,
            next_pending_send_capacity: None,
            send_capacity_inc: false,
            is_pending_open: false,
            next_open: None,
            is_pending_push: false,

            next_pending_accept: None,
            is_pending_accept: false,
            recv_flow,
            in_flight_recv_data: 0,
            next_window_update: None,
            is_pending_window_update: false,
            reset_at: None,
            next_reset_expire: None,
            pending_recv: buffer::Deque::new(),
            recv_task: None,
            pending_push_promises: store::Queue::new(),
            content_length: ContentLength::Omitted,
        }
    }

    pub fn ref_inc(&mut self) {
        assert!(self.ref_count < usize::MAX);
        self.ref_count += 1;
    }

    pub fn ref_dec(&mut self) {
        assert!(self.ref_count > 0);
        self.ref_count -= 1;
    }

    pub fn is_pending_reset_expiration(&self) -> bool {
        self.reset_at.is_some()
    }

    pub fn is_send_ready(&self) -> bool {
        !self.is_pending_open && !self.is_pending_push
    }

    pub fn is_closed(&self) -> bool {
        self.state.is_closed() && self.pending_send.is_empty() && self.buffered_send_data == 0
    }

    pub fn is_released(&self) -> bool {
        self.is_closed()
            && self.ref_count == 0
            && !self.is_pending_send
            && !self.is_pending_send_capacity
            && !self.is_pending_accept
            && !self.is_pending_window_update
            && !self.is_pending_open
            && !self.reset_at.is_some()
    }

    pub fn is_canceled_interest(&self) -> bool {
        self.ref_count == 0 && !self.state.is_closed()
    }

    pub fn assign_capacity(&mut self, capacity: WindowSize) {
        debug_assert!(capacity > 0);
        self.send_capacity_inc = true;
        self.send_flow.assign_capacity(capacity);

        tracing::trace!(
            "  assigned capacity to stream; available={}; buffered={}; id={:?}",
            self.send_flow.available(),
            self.buffered_send_data,
            self.id
        );

        if self.send_flow.available() > self.buffered_send_data {
            tracing::trace!("  notifying task");
            self.notify_send();
        }
    }

    pub fn dec_content_length(&mut self, len: usize) -> Result<(), ()> {
        match self.content_length {
            ContentLength::Remaining(ref mut rem) => match rem.checked_sub(len as u64) {
                Some(val) => *rem = val,
                None => return Err(()),
            },
            ContentLength::Head => {
                if len != 0 {
                    return Err(());
                }
            }
            _ => {}
        }

        Ok(())
    }

    pub fn ensure_content_length_zero(&self) -> Result<(), ()> {
        match self.content_length {
            ContentLength::Remaining(0) => Ok(()),
            ContentLength::Remaining(_) => Err(()),
            _ => Ok(()),
        }
    }

    pub fn notify_send(&mut self) {
        if let Some(task) = self.send_task.take() {
            task.wake();
        }
    }

    pub fn wait_send(&mut self, cx: &Context) {
        self.send_task = Some(cx.waker().clone());
    }

    pub fn notify_recv(&mut self) {
        if let Some(task) = self.recv_task.take() {
            task.wake();
        }
    }
}

impl store::Next for NextAccept {
    fn next(stream: &Stream) -> Option<store::Key> {
        stream.next_pending_accept
    }

    fn set_next(stream: &mut Stream, key: Option<store::Key>) {
        stream.next_pending_accept = key;
    }

    fn take_next(stream: &mut Stream) -> Option<store::Key> {
        stream.next_pending_accept.take()
    }

    fn is_queued(stream: &Stream) -> bool {
        stream.is_pending_accept
    }

    fn set_queued(stream: &mut Stream, val: bool) {
        stream.is_pending_accept = val;
    }
}

impl store::Next for NextSend {
    fn next(stream: &Stream) -> Option<store::Key> {
        stream.next_pending_send
    }

    fn set_next(stream: &mut Stream, key: Option<store::Key>) {
        stream.next_pending_send = key;
    }

    fn take_next(stream: &mut Stream) -> Option<store::Key> {
        stream.next_pending_send.take()
    }

    fn is_queued(stream: &Stream) -> bool {
        stream.is_pending_send
    }

    fn set_queued(stream: &mut Stream, val: bool) {
        if val {
            debug_assert_eq!(stream.is_pending_open, false);
        }
        stream.is_pending_send = val;
    }
}

impl store::Next for NextSendCapacity {
    fn next(stream: &Stream) -> Option<store::Key> {
        stream.next_pending_send_capacity
    }

    fn set_next(stream: &mut Stream, key: Option<store::Key>) {
        stream.next_pending_send_capacity = key;
    }

    fn take_next(stream: &mut Stream) -> Option<store::Key> {
        stream.next_pending_send_capacity.take()
    }

    fn is_queued(stream: &Stream) -> bool {
        stream.is_pending_send_capacity
    }

    fn set_queued(stream: &mut Stream, val: bool) {
        stream.is_pending_send_capacity = val;
    }
}

impl store::Next for NextWindowUpdate {
    fn next(stream: &Stream) -> Option<store::Key> {
        stream.next_window_update
    }

    fn set_next(stream: &mut Stream, key: Option<store::Key>) {
        stream.next_window_update = key;
    }

    fn take_next(stream: &mut Stream) -> Option<store::Key> {
        stream.next_window_update.take()
    }

    fn is_queued(stream: &Stream) -> bool {
        stream.is_pending_window_update
    }

    fn set_queued(stream: &mut Stream, val: bool) {
        stream.is_pending_window_update = val;
    }
}

impl store::Next for NextOpen {
    fn next(stream: &Stream) -> Option<store::Key> {
        stream.next_open
    }

    fn set_next(stream: &mut Stream, key: Option<store::Key>) {
        stream.next_open = key;
    }

    fn take_next(stream: &mut Stream) -> Option<store::Key> {
        stream.next_open.take()
    }

    fn is_queued(stream: &Stream) -> bool {
        stream.is_pending_open
    }

    fn set_queued(stream: &mut Stream, val: bool) {
        if val {
            debug_assert_eq!(stream.is_pending_send, false);
        }
        stream.is_pending_open = val;
    }
}

impl store::Next for NextResetExpire {
    fn next(stream: &Stream) -> Option<store::Key> {
        stream.next_reset_expire
    }

    fn set_next(stream: &mut Stream, key: Option<store::Key>) {
        stream.next_reset_expire = key;
    }

    fn take_next(stream: &mut Stream) -> Option<store::Key> {
        stream.next_reset_expire.take()
    }

    fn is_queued(stream: &Stream) -> bool {
        stream.reset_at.is_some()
    }

    fn set_queued(stream: &mut Stream, val: bool) {
        if val {
            stream.reset_at = Some(Instant::now());
        } else {
            stream.reset_at = None;
        }
    }
}

impl ContentLength {
    pub fn is_head(&self) -> bool {
        match *self {
            ContentLength::Head => true,
            _ => false,
        }
    }
}
