use cynthia::future::Stream;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::common::http_types::upgrade::Connection;

#[must_use = "Futures do nothing unless polled or .awaited"]
#[derive(Debug)]
pub struct Receiver {
    receiver: cynthia::platform::channel::Receiver<Connection>,
}

impl Receiver {
    #[allow(unused)]
    pub(crate) fn new(receiver: cynthia::platform::channel::Receiver<Connection>) -> Self {
        Self { receiver }
    }
}

impl Future for Receiver {
    type Output = Option<Connection>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.receiver).poll_next(cx)
    }
}
