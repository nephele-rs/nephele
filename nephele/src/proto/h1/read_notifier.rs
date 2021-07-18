use cynthia::future::swap::{self, AsyncBufRead, AsyncRead};
use cynthia::platform::channel::Sender;
use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};

#[pin_project::pin_project]
pub(crate) struct ReadNotifier<B> {
    #[pin]
    reader: B,
    sender: Sender<()>,
    has_been_read: bool,
}

impl<B> fmt::Debug for ReadNotifier<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReadNotifier")
            .field("read", &self.has_been_read)
            .finish()
    }
}

impl<B: AsyncRead> ReadNotifier<B> {
    pub(crate) fn new(reader: B, sender: Sender<()>) -> Self {
        Self {
            reader,
            sender,
            has_been_read: false,
        }
    }
}

impl<B: AsyncBufRead> AsyncBufRead for ReadNotifier<B> {
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<swap::Result<&[u8]>> {
        self.project().reader.poll_fill_buf(cx)
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        self.project().reader.consume(amt)
    }
}

impl<B: AsyncRead> AsyncRead for ReadNotifier<B> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<swap::Result<usize>> {
        let this = self.project();

        if !*this.has_been_read {
            if let Ok(()) = this.sender.try_send(()) {
                *this.has_been_read = true;
            };
        }

        this.reader.poll_read(cx, buf)
    }
}
