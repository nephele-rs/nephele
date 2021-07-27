use anyhow::Result;
use std::net::TcpListener;

use cynthia::runtime::{self, future, Async};

use nephele::http_types::{Request, Response, StatusCode};
use nephele::tls::{Identity, TlsAcceptor};

async fn serve(req: Request) -> nephele::common::http_types::Result<Response> {
    println!("Serving {}", req.url());

    let mut res = Response::new(StatusCode::Ok);
    res.insert_header("Content-Type", "text/plain");
    res.set_body("Hello from h1 server!");
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
                let stream = runtime::dup::Arc::new(stream);
                runtime::spawn(async move {
                    if let Err(err) = nephele::proto::h1::accept(stream, serve).await {
                        println!("Connection error: {:#?}", err);
                    }
                })
            }
            Some(tls) => match tls.accept(stream).await {
                Ok(stream) => {
                    let stream = runtime::dup::Arc::new(runtime::dup::Mutex::new(stream));
                    runtime::spawn(async move {
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

    let listener = Async::<TcpListener>::bind("127.0.0.1:7000").await?;
    let http = listen(listener, None);
    let https = listen(
        Async::<TcpListener>::bind("127.0.0.1:8002").await?,
        Some(tls),
    );
    future::try_zip(http, https).await?;
    Ok(())
}
