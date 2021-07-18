use std::fmt;
use std::marker::Unpin;

use native_tls::Error;

use crate::common::tls::handshake::handshake;
use crate::common::tls::runtime::{AsyncRead, AsyncWrite};
use crate::common::tls::TlsStream;

#[derive(Clone)]
pub(crate) struct TlsConnector(native_tls::TlsConnector);

impl TlsConnector {
    pub(crate) async fn connect<S>(&self, domain: &str, stream: S) -> Result<TlsStream<S>, Error>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        handshake(move |s| self.0.connect(domain, s), stream).await
    }
}

impl fmt::Debug for TlsConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsConnector").finish()
    }
}

impl From<native_tls::TlsConnector> for TlsConnector {
    fn from(inner: native_tls::TlsConnector) -> TlsConnector {
        TlsConnector(inner)
    }
}
