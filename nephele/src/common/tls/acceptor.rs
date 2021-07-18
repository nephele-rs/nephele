use std::fmt;
use std::marker::Unpin;

use crate::common::tls::handshake::handshake;
use crate::common::tls::runtime::{AsyncRead, AsyncReadExt, AsyncWrite};
use crate::common::tls::TlsStream;

#[derive(Clone)]
pub struct TlsAcceptor(native_tls::TlsAcceptor);

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("NativeTls({})", 0)]
    NativeTls(#[from] native_tls::Error),
    #[error("Io({})", 0)]
    Io(#[from] std::io::Error),
}

impl TlsAcceptor {
    pub async fn new<R, S>(mut file: R, password: S) -> Result<Self, Error>
    where
        R: AsyncRead + Unpin,
        S: AsRef<str>,
    {
        let mut identity = vec![];
        file.read_to_end(&mut identity).await?;

        let identity = native_tls::Identity::from_pkcs12(&identity, password.as_ref())?;
        Ok(TlsAcceptor(native_tls::TlsAcceptor::new(identity)?))
    }

    pub async fn accept<S>(&self, stream: S) -> Result<TlsStream<S>, native_tls::Error>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let stream = handshake(move |s| self.0.accept(s), stream).await?;
        Ok(stream)
    }
}

impl fmt::Debug for TlsAcceptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsAcceptor").finish()
    }
}

impl From<native_tls::TlsAcceptor> for TlsAcceptor {
    fn from(inner: native_tls::TlsAcceptor) -> TlsAcceptor {
        TlsAcceptor(inner)
    }
}
