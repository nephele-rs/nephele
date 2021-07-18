macro_rules! proto_err {
    (conn: $($msg:tt)+) => {
        tracing::debug!("connection error PROTOCOL_ERROR -- {};", format_args!($($msg)+))
    };
    (stream: $($msg:tt)+) => {
        tracing::debug!("stream error PROTOCOL_ERROR -- {};", format_args!($($msg)+))
    };
}

macro_rules! ready {
    ($e:expr) => {
        match $e {
            ::std::task::Poll::Ready(r) => r,
            ::std::task::Poll::Pending => return ::std::task::Poll::Pending,
        }
    };
}

#[cfg_attr(feature = "unstable", allow(missing_docs))]
mod codec;
mod error;
mod hpack;
mod proto;

#[cfg(not(feature = "unstable"))]
mod frame;

#[cfg(feature = "unstable")]
#[allow(missing_docs)]
pub mod frame;

pub mod client;
pub mod server;
mod share;

pub use crate::proto::h2::error::{Error, Reason};
pub use crate::proto::h2::share::{
    FlowControl, Ping, PingPong, Pong, RecvStream, SendStream, StreamId,
};

#[cfg(feature = "unstable")]
pub use crate::proto::h2::codec::{Codec, RecvError, SendError, UserError};

use std::task::Poll;

trait PollExt<T, E> {
    fn map_ok_<U, F>(self, f: F) -> Poll<Option<Result<U, E>>>
    where
        F: FnOnce(T) -> U;
    fn map_err_<U, F>(self, f: F) -> Poll<Option<Result<T, U>>>
    where
        F: FnOnce(E) -> U;
}

impl<T, E> PollExt<T, E> for Poll<Option<Result<T, E>>> {
    fn map_ok_<U, F>(self, f: F) -> Poll<Option<Result<U, E>>>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            Poll::Ready(Some(Ok(t))) => Poll::Ready(Some(Ok(f(t)))),
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }

    fn map_err_<U, F>(self, f: F) -> Poll<Option<Result<T, U>>>
    where
        F: FnOnce(E) -> U,
    {
        match self {
            Poll::Ready(Some(Ok(t))) => Poll::Ready(Some(Ok(t))),
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(f(e)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
