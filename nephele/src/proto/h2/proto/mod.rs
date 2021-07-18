mod connection;
mod error;
mod go_away;
mod peer;
mod ping_pong;
mod settings;
mod streams;

pub(crate) use crate::proto::h2::proto::connection::{Config, Connection};
pub(crate) use crate::proto::h2::proto::error::Error;
pub(crate) use crate::proto::h2::proto::peer::{Dyn as DynPeer, Peer};
pub(crate) use crate::proto::h2::proto::ping_pong::UserPings;
pub(crate) use crate::proto::h2::proto::streams::{OpaqueStreamRef, StreamRef, Streams};
pub(crate) use crate::proto::h2::proto::streams::{Open, PollReset, Prioritized};

use crate::proto::h2::codec::Codec;

use crate::proto::h2::proto::go_away::GoAway;
use crate::proto::h2::proto::ping_pong::PingPong;
use crate::proto::h2::proto::settings::Settings;

use crate::proto::h2::frame::{self, Frame};

use bytes::Buf;

#[allow(unused_imports)]
use cynthia::future::swap::AsyncWrite;

pub type PingPayload = [u8; 8];

pub type WindowSize = u32;

pub const MAX_WINDOW_SIZE: WindowSize = (1 << 31) - 1;
pub const DEFAULT_RESET_STREAM_MAX: usize = 10;
pub const DEFAULT_RESET_STREAM_SECS: u64 = 30;
