mod buffer;
mod counts;
mod flow_control;
mod prioritize;
mod recv;
mod send;
mod state;
mod store;
mod stream;
mod streams;

use bytes::Bytes;
use std::time::Duration;

pub(crate) use prioritize::Prioritized;
pub(crate) use recv::Open;
pub(crate) use send::PollReset;
pub(crate) use streams::{OpaqueStreamRef, StreamRef, Streams};

use buffer::Buffer;
use counts::Counts;
use flow_control::FlowControl;
use prioritize::Prioritize;
use recv::Recv;
use send::Send;
use state::State;
use store::Store;
use stream::Stream;

use crate::proto::h2::frame::{StreamId, StreamIdOverflow};
use crate::proto::h2::proto::*;

#[derive(Debug)]
pub struct Config {
    pub local_init_window_sz: WindowSize,
    pub initial_max_send_streams: usize,
    pub local_next_stream_id: StreamId,
    pub local_push_enabled: bool,
    pub local_reset_duration: Duration,
    pub local_reset_max: usize,
    pub remote_init_window_sz: WindowSize,
    pub remote_max_initiated: Option<usize>,
}
