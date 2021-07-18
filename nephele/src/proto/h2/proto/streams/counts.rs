use std::usize;

use super::*;

#[derive(Debug)]
pub(super) struct Counts {
    peer: peer::Dyn,
    max_send_streams: usize,
    num_send_streams: usize,
    max_recv_streams: usize,
    num_recv_streams: usize,
    max_reset_streams: usize,
    num_reset_streams: usize,
}

impl Counts {
    pub fn new(peer: peer::Dyn, config: &Config) -> Self {
        Counts {
            peer,
            max_send_streams: config.initial_max_send_streams,
            num_send_streams: 0,
            max_recv_streams: config.remote_max_initiated.unwrap_or(usize::MAX),
            num_recv_streams: 0,
            max_reset_streams: config.local_reset_max,
            num_reset_streams: 0,
        }
    }

    pub fn peer(&self) -> peer::Dyn {
        self.peer
    }

    pub fn has_streams(&self) -> bool {
        self.num_send_streams != 0 || self.num_recv_streams != 0
    }

    pub fn can_inc_num_recv_streams(&self) -> bool {
        self.max_recv_streams > self.num_recv_streams
    }

    pub fn inc_num_recv_streams(&mut self, stream: &mut store::Ptr) {
        assert!(self.can_inc_num_recv_streams());
        assert!(!stream.is_counted);

        self.num_recv_streams += 1;
        stream.is_counted = true;
    }

    pub fn can_inc_num_send_streams(&self) -> bool {
        self.max_send_streams > self.num_send_streams
    }

    pub fn inc_num_send_streams(&mut self, stream: &mut store::Ptr) {
        assert!(self.can_inc_num_send_streams());
        assert!(!stream.is_counted);

        self.num_send_streams += 1;
        stream.is_counted = true;
    }

    pub fn can_inc_num_reset_streams(&self) -> bool {
        self.max_reset_streams > self.num_reset_streams
    }

    pub fn inc_num_reset_streams(&mut self) {
        assert!(self.can_inc_num_reset_streams());

        self.num_reset_streams += 1;
    }

    pub fn apply_remote_settings(&mut self, settings: &frame::Settings) {
        if let Some(val) = settings.max_concurrent_streams() {
            self.max_send_streams = val as usize;
        }
    }

    pub fn transition<F, U>(&mut self, mut stream: store::Ptr, f: F) -> U
    where
        F: FnOnce(&mut Self, &mut store::Ptr) -> U,
    {
        let is_pending_reset = stream.is_pending_reset_expiration();

        let ret = f(self, &mut stream);

        self.transition_after(stream, is_pending_reset);

        ret
    }

    pub fn transition_after(&mut self, mut stream: store::Ptr, is_reset_counted: bool) {
        tracing::trace!(
            "transition_after; stream={:?}; state={:?}; is_closed={:?}; \
             pending_send_empty={:?}; buffered_send_data={}; \
             num_recv={}; num_send={}",
            stream.id,
            stream.state,
            stream.is_closed(),
            stream.pending_send.is_empty(),
            stream.buffered_send_data,
            self.num_recv_streams,
            self.num_send_streams
        );

        if stream.is_closed() {
            if !stream.is_pending_reset_expiration() {
                stream.unlink();
                if is_reset_counted {
                    self.dec_num_reset_streams();
                }
            }

            if stream.is_counted {
                tracing::trace!("dec_num_streams; stream={:?}", stream.id);
                self.dec_num_streams(&mut stream);
            }
        }

        if stream.is_released() {
            stream.remove();
        }
    }

    fn dec_num_streams(&mut self, stream: &mut store::Ptr) {
        assert!(stream.is_counted);

        if self.peer.is_local_init(stream.id) {
            assert!(self.num_send_streams > 0);
            self.num_send_streams -= 1;
            stream.is_counted = false;
        } else {
            assert!(self.num_recv_streams > 0);
            self.num_recv_streams -= 1;
            stream.is_counted = false;
        }
    }

    fn dec_num_reset_streams(&mut self) {
        assert!(self.num_reset_streams > 0);
        self.num_reset_streams -= 1;
    }
}

impl Drop for Counts {
    fn drop(&mut self) {
        use std::thread;

        if !thread::panicking() {
            debug_assert!(!self.has_streams());
        }
    }
}
