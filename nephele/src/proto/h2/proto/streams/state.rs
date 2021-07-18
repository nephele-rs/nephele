use std::io;

use crate::proto::h2::codec::UserError::*;
use crate::proto::h2::codec::{RecvError, UserError};
use crate::proto::h2::frame::Reason;
use crate::proto::h2::proto::{self, PollReset};

use self::Inner::*;
use self::Peer::*;

#[derive(Debug, Clone)]
pub struct State {
    inner: Inner,
}

#[derive(Debug, Clone, Copy)]
enum Inner {
    Idle,
    ReservedLocal,
    ReservedRemote,
    Open { local: Peer, remote: Peer },
    HalfClosedLocal(Peer),
    HalfClosedRemote(Peer),
    Closed(Cause),
}

#[derive(Debug, Copy, Clone)]
enum Peer {
    AwaitingHeaders,
    Streaming,
}

#[derive(Debug, Copy, Clone)]
enum Cause {
    EndStream,
    Proto(Reason),
    LocallyReset(Reason),
    Io,

    Scheduled(Reason),
}

impl State {
    pub fn send_open(&mut self, eos: bool) -> Result<(), UserError> {
        let local = Streaming;

        self.inner = match self.inner {
            Idle => {
                if eos {
                    HalfClosedLocal(AwaitingHeaders)
                } else {
                    Open {
                        local,
                        remote: AwaitingHeaders,
                    }
                }
            }
            Open {
                local: AwaitingHeaders,
                remote,
            } => {
                if eos {
                    HalfClosedLocal(remote)
                } else {
                    Open { local, remote }
                }
            }
            HalfClosedRemote(AwaitingHeaders) | ReservedLocal => {
                if eos {
                    Closed(Cause::EndStream)
                } else {
                    HalfClosedRemote(local)
                }
            }
            _ => {
                return Err(UnexpectedFrameType);
            }
        };

        Ok(())
    }

    pub fn recv_open(&mut self, eos: bool) -> Result<bool, RecvError> {
        let remote = Streaming;
        let mut initial = false;

        self.inner = match self.inner {
            Idle => {
                initial = true;

                if eos {
                    HalfClosedRemote(AwaitingHeaders)
                } else {
                    Open {
                        local: AwaitingHeaders,
                        remote,
                    }
                }
            }
            ReservedRemote => {
                initial = true;

                if eos {
                    Closed(Cause::EndStream)
                } else {
                    HalfClosedLocal(Streaming)
                }
            }
            Open {
                local,
                remote: AwaitingHeaders,
            } => {
                if eos {
                    HalfClosedRemote(local)
                } else {
                    Open { local, remote }
                }
            }
            HalfClosedLocal(AwaitingHeaders) => {
                if eos {
                    Closed(Cause::EndStream)
                } else {
                    HalfClosedLocal(remote)
                }
            }
            state => {
                proto_err!(conn: "recv_open: in unexpected state {:?}", state);
                return Err(RecvError::Connection(Reason::PROTOCOL_ERROR));
            }
        };

        Ok(initial)
    }

    pub fn reserve_remote(&mut self) -> Result<(), RecvError> {
        match self.inner {
            Idle => {
                self.inner = ReservedRemote;
                Ok(())
            }
            state => {
                proto_err!(conn: "reserve_remote: in unexpected state {:?}", state);
                Err(RecvError::Connection(Reason::PROTOCOL_ERROR))
            }
        }
    }

    pub fn reserve_local(&mut self) -> Result<(), UserError> {
        match self.inner {
            Idle => {
                self.inner = ReservedLocal;
                Ok(())
            }
            _ => Err(UserError::UnexpectedFrameType),
        }
    }

    pub fn recv_close(&mut self) -> Result<(), RecvError> {
        match self.inner {
            Open { local, .. } => {
                tracing::trace!("recv_close: Open => HalfClosedRemote({:?})", local);
                self.inner = HalfClosedRemote(local);
                Ok(())
            }
            HalfClosedLocal(..) => {
                tracing::trace!("recv_close: HalfClosedLocal => Closed");
                self.inner = Closed(Cause::EndStream);
                Ok(())
            }
            state => {
                proto_err!(conn: "recv_close: in unexpected state {:?}", state);
                Err(RecvError::Connection(Reason::PROTOCOL_ERROR))
            }
        }
    }

    pub fn recv_reset(&mut self, reason: Reason, queued: bool) {
        match self.inner {
            Closed(..) if !queued => {}
            state => {
                tracing::trace!(
                    "recv_reset; reason={:?}; state={:?}; queued={:?}",
                    reason,
                    state,
                    queued
                );
                self.inner = Closed(Cause::Proto(reason));
            }
        }
    }

    pub fn recv_err(&mut self, err: &proto::Error) {
        use crate::proto::h2::proto::Error::*;

        match self.inner {
            Closed(..) => {}
            _ => {
                tracing::trace!("recv_err; err={:?}", err);
                self.inner = Closed(match *err {
                    Proto(reason) => Cause::LocallyReset(reason),
                    Io(..) => Cause::Io,
                });
            }
        }
    }

    pub fn recv_eof(&mut self) {
        match self.inner {
            Closed(..) => {}
            s => {
                tracing::trace!("recv_eof; state={:?}", s);
                self.inner = Closed(Cause::Io);
            }
        }
    }

    pub fn send_close(&mut self) {
        match self.inner {
            Open { remote, .. } => {
                tracing::trace!("send_close: Open => HalfClosedLocal({:?})", remote);
                self.inner = HalfClosedLocal(remote);
            }
            HalfClosedRemote(..) => {
                tracing::trace!("send_close: HalfClosedRemote => Closed");
                self.inner = Closed(Cause::EndStream);
            }
            state => panic!("send_close: unexpected state {:?}", state),
        }
    }

    pub fn set_reset(&mut self, reason: Reason) {
        self.inner = Closed(Cause::LocallyReset(reason));
    }

    pub fn set_scheduled_reset(&mut self, reason: Reason) {
        debug_assert!(!self.is_closed());
        self.inner = Closed(Cause::Scheduled(reason));
    }

    pub fn get_scheduled_reset(&self) -> Option<Reason> {
        match self.inner {
            Closed(Cause::Scheduled(reason)) => Some(reason),
            _ => None,
        }
    }

    pub fn is_scheduled_reset(&self) -> bool {
        match self.inner {
            Closed(Cause::Scheduled(..)) => true,
            _ => false,
        }
    }

    pub fn is_local_reset(&self) -> bool {
        match self.inner {
            Closed(Cause::LocallyReset(_)) => true,
            Closed(Cause::Scheduled(..)) => true,
            _ => false,
        }
    }

    pub fn is_reset(&self) -> bool {
        match self.inner {
            Closed(Cause::EndStream) => false,
            Closed(_) => true,
            _ => false,
        }
    }

    pub fn is_send_streaming(&self) -> bool {
        match self.inner {
            Open {
                local: Streaming, ..
            } => true,
            HalfClosedRemote(Streaming) => true,
            _ => false,
        }
    }

    pub fn is_recv_headers(&self) -> bool {
        match self.inner {
            Idle => true,
            Open {
                remote: AwaitingHeaders,
                ..
            } => true,
            HalfClosedLocal(AwaitingHeaders) => true,
            ReservedRemote => true,
            _ => false,
        }
    }

    pub fn is_recv_streaming(&self) -> bool {
        match self.inner {
            Open {
                remote: Streaming, ..
            } => true,
            HalfClosedLocal(Streaming) => true,
            _ => false,
        }
    }

    pub fn is_closed(&self) -> bool {
        match self.inner {
            Closed(_) => true,
            _ => false,
        }
    }

    pub fn is_recv_closed(&self) -> bool {
        match self.inner {
            Closed(..) | HalfClosedRemote(..) | ReservedLocal => true,
            _ => false,
        }
    }

    pub fn is_send_closed(&self) -> bool {
        match self.inner {
            Closed(..) | HalfClosedLocal(..) | ReservedRemote => true,
            _ => false,
        }
    }

    pub fn is_idle(&self) -> bool {
        match self.inner {
            Idle => true,
            _ => false,
        }
    }

    pub fn ensure_recv_open(&self) -> Result<bool, proto::Error> {
        match self.inner {
            Closed(Cause::Proto(reason))
            | Closed(Cause::LocallyReset(reason))
            | Closed(Cause::Scheduled(reason)) => Err(proto::Error::Proto(reason)),
            Closed(Cause::Io) => Err(proto::Error::Io(io::ErrorKind::BrokenPipe.into())),
            Closed(Cause::EndStream) | HalfClosedRemote(..) | ReservedLocal => Ok(false),
            _ => Ok(true),
        }
    }

    pub(super) fn ensure_reason(
        &self,
        mode: PollReset,
    ) -> Result<Option<Reason>, crate::proto::h2::Error> {
        match self.inner {
            Closed(Cause::Proto(reason))
            | Closed(Cause::LocallyReset(reason))
            | Closed(Cause::Scheduled(reason)) => Ok(Some(reason)),
            Closed(Cause::Io) => Err(proto::Error::Io(io::ErrorKind::BrokenPipe.into()).into()),
            Open {
                local: Streaming, ..
            }
            | HalfClosedRemote(Streaming) => match mode {
                PollReset::AwaitingHeaders => Err(UserError::PollResetAfterSendResponse.into()),
                PollReset::Streaming => Ok(None),
            },
            _ => Ok(None),
        }
    }
}

impl Default for State {
    fn default() -> State {
        State { inner: Inner::Idle }
    }
}

impl Default for Peer {
    fn default() -> Self {
        AwaitingHeaders
    }
}
