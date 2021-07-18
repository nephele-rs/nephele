use std::fmt::{self, Debug};
use std::pin::Pin;
use std::task::{Context, Poll};

use cynthia::future::{prelude::*, swap};

pub struct Connection {
    inner: Box<dyn InnerConnection>,
}

impl Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = "Box<dyn Asyncread + AsyncWrite + Send + Sync + Unpin>";
        f.debug_struct("Connection")
            .field(&"inner", &inner)
            .finish()
    }
}

impl Connection {
    pub fn new<T>(t: T) -> Self
    where
        T: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static,
    {
        Self { inner: Box::new(t) }
    }
}

pub trait InnerConnection: AsyncRead + AsyncWrite + Send + Sync + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Sync + Unpin> InnerConnection for T {}

impl AsyncRead for Connection {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<swap::Result<usize>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for Connection {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<swap::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<swap::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<swap::Result<()>> {
        Pin::new(&mut self.inner).poll_close(cx)
    }
}
