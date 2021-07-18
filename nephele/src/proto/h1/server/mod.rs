use cynthia::future::swap::{self, AsyncRead, AsyncWrite};
use cynthia::future::{timeout, Future, TimeoutError};
use std::{marker::PhantomData, time::Duration};

use crate::common::http_types::headers::{CONNECTION, UPGRADE};
use crate::common::http_types::upgrade::Connection;
use crate::common::http_types::{Request, Response, StatusCode};

mod body_reader;
mod decode;
mod encode;

pub use decode::decode;
pub use encode::Encoder;

#[derive(Debug, Clone)]
pub struct ServerOptions {
    headers_timeout: Option<Duration>,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            headers_timeout: Some(Duration::from_secs(60)),
        }
    }
}

pub async fn accept<RW, F, Fut>(io: RW, endpoint: F) -> crate::common::http_types::Result<()>
where
    RW: AsyncRead + AsyncWrite + Clone + Send + Sync + Unpin + 'static,
    F: Fn(Request) -> Fut,
    Fut: Future<Output = crate::common::http_types::Result<Response>>,
{
    Server::new(io, endpoint).accept().await
}

pub async fn accept_with_opts<RW, F, Fut>(
    io: RW,
    endpoint: F,
    opts: ServerOptions,
) -> crate::common::http_types::Result<()>
where
    RW: AsyncRead + AsyncWrite + Clone + Send + Sync + Unpin + 'static,
    F: Fn(Request) -> Fut,
    Fut: Future<Output = crate::common::http_types::Result<Response>>,
{
    Server::new(io, endpoint).with_opts(opts).accept().await
}

#[derive(Debug)]
pub struct Server<RW, F, Fut> {
    io: RW,
    endpoint: F,
    opts: ServerOptions,
    _phantom: PhantomData<Fut>,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ConnectionStatus {
    Close,
    KeepAlive,
}

impl<RW, F, Fut> Server<RW, F, Fut>
where
    RW: AsyncRead + AsyncWrite + Clone + Send + Sync + Unpin + 'static,
    F: Fn(Request) -> Fut,
    Fut: Future<Output = crate::common::http_types::Result<Response>>,
{
    pub fn new(io: RW, endpoint: F) -> Self {
        Self {
            io,
            endpoint,
            opts: Default::default(),
            _phantom: PhantomData,
        }
    }

    pub fn with_opts(mut self, opts: ServerOptions) -> Self {
        self.opts = opts;
        self
    }

    pub async fn accept(&mut self) -> crate::common::http_types::Result<()> {
        while ConnectionStatus::KeepAlive == self.accept_one().await? {}
        Ok(())
    }

    pub async fn accept_one(&mut self) -> crate::common::http_types::Result<ConnectionStatus>
    where
        RW: AsyncRead + AsyncWrite + Clone + Send + Sync + Unpin + 'static,
        F: Fn(Request) -> Fut,
        Fut: Future<Output = crate::common::http_types::Result<Response>>,
    {
        let fut = decode(self.io.clone());

        let (req, mut body) = if let Some(timeout_duration) = self.opts.headers_timeout {
            match timeout(timeout_duration, fut).await {
                Ok(Ok(Some(r))) => r,
                Ok(Ok(None)) | Err(TimeoutError { .. }) => return Ok(ConnectionStatus::Close), /* EOF or timeout */
                Ok(Err(e)) => return Err(e),
            }
        } else {
            match fut.await? {
                Some(r) => r,
                None => return Ok(ConnectionStatus::Close), /* EOF */
            }
        };

        let has_upgrade_header = req.header(UPGRADE).is_some();
        let connection_header_as_str = req
            .header(CONNECTION)
            .map(|connection| connection.as_str())
            .unwrap_or("");

        let connection_header_is_upgrade = connection_header_as_str.eq_ignore_ascii_case("upgrade");
        let mut close_connection = connection_header_as_str.eq_ignore_ascii_case("close");

        let upgrade_requested = has_upgrade_header && connection_header_is_upgrade;

        let method = req.method();

        let mut res = (self.endpoint)(req).await?;

        close_connection |= res
            .header(CONNECTION)
            .map(|c| c.as_str().eq_ignore_ascii_case("close"))
            .unwrap_or(false);

        let upgrade_provided = res.status() == StatusCode::SwitchingProtocols && res.has_upgrade();

        let upgrade_sender = if upgrade_requested && upgrade_provided {
            Some(res.send_upgrade())
        } else {
            None
        };

        let mut encoder = Encoder::new(res, method);

        let _bytes_written = swap::copy(&mut encoder, &mut self.io).await?;

        let _body_bytes_discarded = swap::copy(&mut body, &mut swap::sink()).await?;

        if let Some(upgrade_sender) = upgrade_sender {
            upgrade_sender.send(Connection::new(self.io.clone())).await;
            return Ok(ConnectionStatus::Close);
        } else if close_connection {
            Ok(ConnectionStatus::Close)
        } else {
            Ok(ConnectionStatus::KeepAlive)
        }
    }
}
