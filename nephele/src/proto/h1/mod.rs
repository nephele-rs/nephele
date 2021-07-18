#![forbid(unsafe_code)]

const MAX_HEADERS: usize = 128;

const MAX_HEAD_LENGTH: usize = 8 * 1024;

mod body_encoder;
mod chunked;
mod date;
mod read_notifier;

pub mod client;
pub mod server;

use crate::proto::h1::body_encoder::BodyEncoder;
use cynthia::future::swap::Cursor;

pub use client::connect;
pub use server::{accept, accept_with_opts, ServerOptions};

#[derive(Debug)]
pub(crate) enum EncoderState {
    Start,
    Head(Cursor<Vec<u8>>),
    Body(BodyEncoder),
    End,
}

#[macro_export]
macro_rules! read_to_end {
    ($expr:expr) => {
        match $expr {
            Poll::Ready(Ok(0)) => (),
            other => return other,
        }
    };
}
