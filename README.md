# nephele
A relatively low-level Rust framework for libraries and applications.

# Overview
A fast and simple HTTP implementation for Rust.
* HTTP/1 and HTTP/2
* Asynchronous design
* Client and Server APIs

# Example
Make sure you activated the full features of the nephele crate on Cargo.toml:
```toml
[dependencies]
nephele = { version = "0.0.2" }
```

A basic Http echo server with nephele.
```rust,no_run
use anyhow::Result;
use std::net::TcpListener;

use cynthia::runtime::{self, future, Async};

use nephele::http_types::{Request, Response, StatusCode};

async fn serve(req: Request) -> nephele::common::http_types::Result<Response> {
    println!("Serving {}", req.url());

    let mut res = Response::new(StatusCode::Ok);
    res.insert_header("Content-Type", "text/plain");
    res.set_body("Hello from h1 server!");
    Ok(res)
}

async fn listen(listener: Async<TcpListener>) -> Result<()> {
    let host = format!("https://{}", listener.get_ref().local_addr()?);
    println!("Listening on {}", host);

    loop {
        let (stream, _) = listener.accept().await?;
        let stream = runtime::dup::Arc::new(stream);
        let task = runtime::spawn(async move {
             if let Err(err) = nephele::proto::h1::accept(stream, serve).await {
                println!("Connection error: {:#?}", err);
             }
        });

        task.detach();
    }
}

#[cynthia::main]
async fn main() -> Result<()> {
    let listener = Async::<TcpListener>::bind("127.0.0.1:7000").await?;
    let http = listen(listener);
    http.await?;
    Ok(())
}
```
More examples can be found [here](https://github.com/nephele-rs/nephele/tree/main/examples).

# Supported Rust Versions
This library is verified to work in rustc 1.51.0 (nightly), and the support of other versions needs more testing.

# License
This project is licensed under the [Apache License 2.0](https://github.com/nephele-rs/nephele/blob/main/LICENSE).
