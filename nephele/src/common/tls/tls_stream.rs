use std::io::{self, Read, Write};
use std::marker::Unpin;
use std::pin::Pin;
use std::ptr::null_mut;
use std::task::{Context, Poll};

use crate::common::tls::runtime::{AsyncRead, AsyncWrite};
use crate::common::tls::std_adapter::StdAdapter;

#[derive(Debug)]
pub struct TlsStream<S>(native_tls::TlsStream<StdAdapter<S>>);

impl<S> TlsStream<S> {
    pub(crate) fn new(stream: native_tls::TlsStream<StdAdapter<S>>) -> Self {
        Self(stream)
    }
    fn with_context<F, R>(&mut self, ctx: &mut Context<'_>, f: F) -> R
    where
        F: FnOnce(&mut native_tls::TlsStream<StdAdapter<S>>) -> R,
        StdAdapter<S>: Read + Write,
    {
        self.0.get_mut().context = ctx as *mut _ as *mut ();
        let g = Guard(self);
        f(&mut (g.0).0)
    }

    pub fn get_ref(&self) -> &S
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        &self.0.get_ref().inner
    }

    pub fn get_mut(&mut self) -> &mut S
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        &mut self.0.get_mut().inner
    }

    pub fn buffered_read_size(&self) -> crate::common::tls::Result<usize>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        self.0.buffered_read_size()
    }

    pub fn peer_certificate(
        &self,
    ) -> crate::common::tls::Result<Option<crate::common::tls::Certificate>>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        self.0.peer_certificate()
    }

    pub fn tls_server_end_point(&self) -> crate::common::tls::Result<Option<Vec<u8>>>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        self.0.tls_server_end_point()
    }
}

impl<S> AsyncRead for TlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        self.with_context(ctx, |s| cvt(s.read(buf)))
    }
}

impl<S> AsyncWrite for TlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.with_context(ctx, |s| cvt(s.write(buf)))
    }

    fn poll_flush(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.with_context(ctx, |s| cvt(s.flush()))
    }

    fn poll_close(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.with_context(ctx, |s| s.shutdown()) {
            Ok(()) => Poll::Ready(Ok(())),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    #[cfg(feature = "runtime-tokio")]
    fn poll_shutdown(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.with_context(ctx, |s| s.shutdown()) {
            Ok(()) => Poll::Ready(Ok(())),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

struct Guard<'a, S>(&'a mut TlsStream<S>)
where
    StdAdapter<S>: Read + Write;

impl<S> Drop for Guard<'_, S>
where
    StdAdapter<S>: Read + Write,
{
    fn drop(&mut self) {
        (self.0).0.get_mut().context = null_mut();
    }
}

fn cvt<T>(r: io::Result<T>) -> Poll<io::Result<T>> {
    match r {
        Ok(v) => Poll::Ready(Ok(v)),
        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
        Err(e) => Poll::Ready(Err(e)),
    }
}
