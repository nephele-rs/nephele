#![warn(missing_debug_implementations)]

mod acceptor;
mod connector;
mod handshake;
mod runtime;
mod std_adapter;
mod tls_stream;

pub use accept::accept;
pub use acceptor::{Error as AcceptError, TlsAcceptor};
pub use connect::{connect, TlsConnector};
pub use host::Host;
pub use tls_stream::TlsStream;

#[doc(inline)]
pub use native_tls::{Certificate, Error, Identity, Protocol, Result};

mod accept {
    use crate::common::tls::runtime::{AsyncRead, AsyncWrite};

    use crate::common::tls::TlsStream;

    pub async fn accept<R, S, T>(
        file: R,
        password: S,
        stream: T,
    ) -> Result<TlsStream<T>, crate::common::tls::AcceptError>
    where
        R: AsyncRead + Unpin,
        S: AsRef<str>,
        T: AsyncRead + AsyncWrite + Unpin,
    {
        let acceptor = crate::common::tls::TlsAcceptor::new(file, password).await?;
        let stream = acceptor.accept(stream).await?;

        Ok(stream)
    }
}

mod host {
    use url::Url;

    #[derive(Debug)]
    pub struct Host(String);

    impl Host {
        #[allow(clippy::wrong_self_convention)]
        pub fn as_string(self) -> String {
            self.0
        }
    }

    impl From<&str> for Host {
        fn from(host: &str) -> Self {
            Self(host.into())
        }
    }

    impl From<String> for Host {
        fn from(host: String) -> Self {
            Self(host)
        }
    }

    impl From<&String> for Host {
        fn from(host: &String) -> Self {
            Self(host.into())
        }
    }

    impl From<Url> for Host {
        fn from(url: Url) -> Self {
            Self(
                url.host_str()
                    .expect("URL has to include a host part.")
                    .into(),
            )
        }
    }

    impl From<&Url> for Host {
        fn from(url: &Url) -> Self {
            Self(
                url.host_str()
                    .expect("URL has to include a host part.")
                    .into(),
            )
        }
    }
}

mod connect {
    use std::fmt::{self, Debug};

    use crate::common::tls::host::Host;
    use crate::common::tls::runtime::{AsyncRead, AsyncWrite};
    use crate::common::tls::TlsStream;
    use crate::common::tls::{Certificate, Identity, Protocol};

    pub async fn connect<S>(host: impl Into<Host>, stream: S) -> native_tls::Result<TlsStream<S>>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let stream = TlsConnector::new().connect(host, stream).await?;
        Ok(stream)
    }

    pub struct TlsConnector {
        builder: native_tls::TlsConnectorBuilder,
    }

    impl Default for TlsConnector {
        fn default() -> Self {
            TlsConnector::new()
        }
    }

    impl TlsConnector {
        pub fn new() -> Self {
            Self {
                builder: native_tls::TlsConnector::builder(),
            }
        }

        pub fn identity(mut self, identity: Identity) -> Self {
            self.builder.identity(identity);
            self
        }

        pub fn min_protocol_version(mut self, protocol: Option<Protocol>) -> Self {
            self.builder.min_protocol_version(protocol);
            self
        }

        pub fn max_protocol_version(mut self, protocol: Option<Protocol>) -> Self {
            self.builder.max_protocol_version(protocol);
            self
        }

        pub fn add_root_certificate(mut self, cert: Certificate) -> Self {
            self.builder.add_root_certificate(cert);
            self
        }

        pub fn danger_accept_invalid_certs(mut self, accept_invalid_certs: bool) -> Self {
            self.builder
                .danger_accept_invalid_certs(accept_invalid_certs);
            self
        }

        pub fn use_sni(mut self, use_sni: bool) -> Self {
            self.builder.use_sni(use_sni);
            self
        }

        pub fn danger_accept_invalid_hostnames(mut self, accept_invalid_hostnames: bool) -> Self {
            self.builder
                .danger_accept_invalid_hostnames(accept_invalid_hostnames);
            self
        }

        pub async fn connect<S>(
            &self,
            host: impl Into<Host>,
            stream: S,
        ) -> native_tls::Result<TlsStream<S>>
        where
            S: AsyncRead + AsyncWrite + Unpin,
        {
            let host: Host = host.into();
            let domain = host.as_string();
            let connector = self.builder.build()?;
            let connector = crate::common::tls::connector::TlsConnector::from(connector);
            let stream = connector.connect(&domain, stream).await?;
            Ok(stream)
        }
    }

    impl Debug for TlsConnector {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("TlsConnector").finish()
        }
    }

    impl From<native_tls::TlsConnectorBuilder> for TlsConnector {
        fn from(builder: native_tls::TlsConnectorBuilder) -> Self {
            Self { builder }
        }
    }
}
