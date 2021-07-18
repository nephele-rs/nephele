use cynthia::future::swap::AsyncRead;
use pin_project::pin_project;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::common::http_types::Body;
use crate::proto::h1::chunked::ChunkedEncoder;

#[pin_project(project=BodyEncoderProjection)]
#[derive(Debug)]
pub(crate) enum BodyEncoder {
    Chunked(#[pin] ChunkedEncoder<Body>),
    Fixed(#[pin] Body),
}

impl BodyEncoder {
    pub(crate) fn new(body: Body) -> Self {
        match body.len() {
            Some(_) => Self::Fixed(body),
            None => Self::Chunked(ChunkedEncoder::new(body)),
        }
    }
}

impl AsyncRead for BodyEncoder {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        match self.project() {
            BodyEncoderProjection::Chunked(encoder) => encoder.poll_read(cx, buf),
            BodyEncoderProjection::Fixed(body) => body.poll_read(cx, buf),
        }
    }
}
