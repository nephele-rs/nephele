use bytes::Buf;
use cynthia::future::swap::AsyncWrite;
use std::io;
use std::task::{Context, Poll};

use crate::proto::h2::codec::Codec;
use crate::proto::h2::frame::{self, Reason, StreamId};

#[derive(Debug)]
pub(super) struct GoAway {
    close_now: bool,
    going_away: Option<GoingAway>,
    is_user_initiated: bool,
    pending: Option<frame::GoAway>,
}

#[derive(Debug)]
struct GoingAway {
    last_processed_id: StreamId,

    reason: Reason,
}

impl GoAway {
    pub fn new() -> Self {
        GoAway {
            close_now: false,
            going_away: None,
            is_user_initiated: false,
            pending: None,
        }
    }

    pub fn go_away(&mut self, f: frame::GoAway) {
        if let Some(ref going_away) = self.going_away {
            assert!(
                f.last_stream_id() <= going_away.last_processed_id,
                "GOAWAY stream IDs shouldn't be higher; \
                 last_processed_id = {:?}, f.last_stream_id() = {:?}",
                going_away.last_processed_id,
                f.last_stream_id(),
            );
        }

        self.going_away = Some(GoingAway {
            last_processed_id: f.last_stream_id(),
            reason: f.reason(),
        });
        self.pending = Some(f);
    }

    pub fn go_away_now(&mut self, f: frame::GoAway) {
        self.close_now = true;
        if let Some(ref going_away) = self.going_away {
            if going_away.last_processed_id == f.last_stream_id() && going_away.reason == f.reason()
            {
                return;
            }
        }
        self.go_away(f);
    }

    pub fn go_away_from_user(&mut self, f: frame::GoAway) {
        self.is_user_initiated = true;
        self.go_away_now(f);
    }

    pub fn is_going_away(&self) -> bool {
        self.going_away.is_some()
    }

    pub fn is_user_initiated(&self) -> bool {
        self.is_user_initiated
    }

    pub fn going_away_reason(&self) -> Option<Reason> {
        self.going_away.as_ref().map(|g| g.reason)
    }

    pub fn should_close_now(&self) -> bool {
        self.pending.is_none() && self.close_now
    }

    pub fn should_close_on_idle(&self) -> bool {
        !self.close_now
            && self
                .going_away
                .as_ref()
                .map(|g| g.last_processed_id != StreamId::MAX)
                .unwrap_or(false)
    }

    pub fn send_pending_go_away<T, B>(
        &mut self,
        cx: &mut Context,
        dst: &mut Codec<T, B>,
    ) -> Poll<Option<io::Result<Reason>>>
    where
        T: AsyncWrite + Unpin,
        B: Buf,
    {
        if let Some(frame) = self.pending.take() {
            if !dst.poll_ready(cx)?.is_ready() {
                self.pending = Some(frame);
                return Poll::Pending;
            }

            let reason = frame.reason();
            dst.buffer(frame.into()).expect("invalid GOAWAY frame");

            return Poll::Ready(Some(Ok(reason)));
        } else if self.should_close_now() {
            return match self.going_away_reason() {
                Some(reason) => Poll::Ready(Some(Ok(reason))),
                None => Poll::Ready(None),
            };
        }

        Poll::Ready(None)
    }
}
