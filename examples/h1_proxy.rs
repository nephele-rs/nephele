use anyhow::{bail, Context as _, Error, Result};
use std::net::{TcpListener, TcpStream};

use cynthia::runtime::{future, Async};

use nephele::http_types::{Request, Response};
use nephele::tls::{Identity, TlsAcceptor};

async fn fetch(req: Request) -> Result<Response> {
    let host = req.url().host().context("cannot parse host")?.to_string();
    println!("host = {:}", host);
    let mut port = req
        .url()
        .port_or_known_default()
        .context("cannot guess port")?;

    println!("port = {:}", port);
    port = 7000;

    let upstream_addr = format!("{}:{}", host.as_str(), port);
    let stream = Async::<TcpStream>::connect(upstream_addr).await?;

    let resp = match req.url().scheme() {
        "http" => nephele::proto::h1::connect(stream, req)
            .await
            .map_err(Error::msg)?,
        "https" => {
            let stream = nephele::common::tls::connect(&host, stream).await?;
            nephele::proto::h1::connect(stream, req)
                .await
                .map_err(Error::msg)?
        }
        scheme => bail!("unsupported scheme: {}", scheme),
    };
    Ok(resp)
}

async fn serve(req: Request) -> nephele::common::http_types::Result<Response> {
    let res = fetch(req).await?;
    Ok(res)
}

async fn listen(listener: Async<TcpListener>, tls: Option<TlsAcceptor>) -> Result<()> {
    let host = match &tls {
        None => format!("http://{}", listener.get_ref().local_addr()?),
        Some(_) => format!("https://{}", listener.get_ref().local_addr()?),
    };
    println!("Listening on {}", host);

    loop {
        let (stream, _) = listener.accept().await?;

        let task = match &tls {
            None => {
                let stream = cynthia::platform::dup::Arc::new(stream);
                cynthia::runtime::spawn(async move {
                    if let Err(err) = nephele::proto::h1::accept(stream, serve).await {
                        println!("Connection error: {:#?}", err);
                    }
                })
            }
            Some(tls) => match tls.accept(stream).await {
                Ok(stream) => {
                    let stream = cynthia::platform::dup::Arc::new(
                        cynthia::platform::dup::Mutex::new(stream),
                    );
                    cynthia::runtime::spawn(async move {
                        if let Err(err) = nephele::proto::h1::accept(stream, serve).await {
                            println!("Connection error: {:#?}", err);
                        }
                    })
                }
                Err(err) => {
                    println!("Failed to establish secure TLS connection: {:#?}", err);
                    continue;
                }
            },
        };

        task.detach();
    }
}

#[cynthia::main]
async fn main() -> Result<()> {
    let identity = Identity::from_pkcs12(include_bytes!("identity.pfx"), "password")?;
    let tls = TlsAcceptor::from(native_tls::TlsAcceptor::new(identity)?);

    let listener = Async::<TcpListener>::bind("127.0.0.1:8000").await?;
    let http = listen(listener, None);

    let https = listen(
        Async::<TcpListener>::bind("127.0.0.1:8001").await?,
        Some(tls),
    );
    future::try_zip(http, https).await?;
    Ok(())
}
