use anyhow::{bail, Context as _, Error, Result};
use std::net::{TcpStream, ToSocketAddrs};
use url::Url;

use cynthia::runtime::{prelude::*, Async};
use nephele::http_types::{Method, Request, Response};

async fn fetch(req: Request) -> Result<Response> {
    let host = req.url().host().context("cannot parse host")?.to_string();
    let port = req
        .url()
        .port_or_known_default()
        .context("cannot guess port")?;

    let socket_addr = {
        let host = host.clone();
        cynthia::runtime::unblock(move || (host.as_str(), port).to_socket_addrs())
            .await?
            .next()
            .context("cannot resolve address")?
    };
    let stream = Async::<TcpStream>::connect(socket_addr).await?;

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

fn main() -> Result<()> {
    cynthia::runtime::block_on(async {
        let addr = "https://www.rust-lang.org";
        let req = Request::new(Method::Get, Url::parse(addr)?);

        let mut resp = fetch(req).await?;

        let mut body = Vec::new();
        resp.read_to_end(&mut body).await?;
        println!("{}", String::from_utf8_lossy(&body));

        Ok(())
    })
}
