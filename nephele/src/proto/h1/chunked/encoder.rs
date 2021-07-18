use core::task::{Context, Poll};
use cynthia::future::prelude::*;
use cynthia::future::swap;
use futures_core::ready;
use std::pin::Pin;

#[derive(Debug)]
pub(crate) struct ChunkedEncoder<R> {
    reader: R,
    done: bool,
}

impl<R: AsyncRead + Unpin> ChunkedEncoder<R> {
    pub(crate) fn new(reader: R) -> Self {
        Self {
            reader,
            done: false,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for ChunkedEncoder<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<swap::Result<usize>> {
        if self.done {
            return Poll::Ready(Ok(0));
        }
        let reader = &mut self.reader;

        let max_bytes_to_read = max_bytes_to_read(buf.len());

        let bytes = ready!(Pin::new(reader).poll_read(cx, &mut buf[..max_bytes_to_read]))?;
        if bytes == 0 {
            self.done = true;
        }
        let start = format!("{:X}\r\n", bytes);
        let start_length = start.as_bytes().len();
        let total = bytes + start_length + 2;
        buf.copy_within(..bytes, start_length);
        buf[..start_length].copy_from_slice(start.as_bytes());
        buf[total - 2..total].copy_from_slice(b"\r\n");
        Poll::Ready(Ok(total))
    }
}

fn max_bytes_to_read(buf_len: usize) -> usize {
    if buf_len < 6 {
        panic!("buffers of length {} are too small for this implementation. if this is a problem for you, please open an issue", buf_len);
    }

    let bytes_remaining_after_two_cr_lns = (buf_len - 4) as f64;

    let max_bytes_of_hex_framing = bytes_remaining_after_two_cr_lns.log2() / 4f64;

    (bytes_remaining_after_two_cr_lns - max_bytes_of_hex_framing.ceil()) as usize
}
