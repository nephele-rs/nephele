use anyhow::{Error, Result};
use std::net::{TcpListener, TcpStream};
use url::Url;

use cynthia::runtime::{self, future, Async};

use nephele::http_types::{convert::json, Body, Method, Request, Response, StatusCode};
use nephele::tls::{Identity, TlsAcceptor};

static INDEX_STR: &[u8] = b"<a href=\"test.html\">test.html</a>";
static INTERNAL_SERVER_ERROR: &[u8] = b"Internal Server Error";
static NOTFOUND_STR: &[u8] = b"Not Found";
static UPSTREAM_URL: &str = "http://127.0.0.1:7000/json_api";

async fn api_fetch(_req: Request) -> nephele::common::http_types::Result<Response> {
    let mut res = Response::new(StatusCode::Ok);
    res.insert_header("Content-Type", "text/plain");
    res.set_body("api_fetch");
    Ok(res)
}

async fn api_fetch_upstream() -> nephele::common::http_types::Result<Response> {
    let mut req = Request::new(Method::Post, Url::parse(UPSTREAM_URL)?);
    req.insert_header("Content-Type", "application/json");

    match Body::from_json(&json!({ "hello": "json" })) {
        Ok(json) => {
            req.set_body(json);
            let stream = Async::<TcpStream>::connect(([127, 0, 0, 1], 7000)).await?;
            let resp = nephele::proto::h1::connect(stream, req)
                .await
                .map_err(Error::msg)?;
            Ok(resp)
        }
        Err(_) => {
            let mut res = Response::new(StatusCode::ServiceUnavailable);
            res.insert_header("Content-Type", "text/plain");
            res.set_body(Body::from_bytes(INTERNAL_SERVER_ERROR.into()));
            Ok(res)
        }
    }
}

async fn api_post_json(mut req: Request) -> nephele::common::http_types::Result<Response> {
    let whole_body = req.take_body();
    let mut data: serde_json::Value = serde_json::from_str(&whole_body.into_string().await?)?;
    data["test"] = serde_json::Value::from("test_value");
    let json = serde_json::to_string(&data)?;

    let mut res = Response::new(StatusCode::Ok);
    res.insert_header("Content-Type", "application/json");
    res.set_body(Body::from(json));
    Ok(res)
}

async fn api_get_json() -> nephele::common::http_types::Result<Response> {
    let data = vec!["foo", "bar"];
    match serde_json::to_string(&data) {
        Ok(json) => {
            let mut res = Response::new(StatusCode::Ok);
            res.insert_header("Content-Type", "application/json");
            res.set_body(Body::from(json));
            Ok(res)
        }
        Err(_) => {
            let mut res = Response::new(StatusCode::ServiceUnavailable);
            res.insert_header("Content-Type", "text/plain");
            res.set_body(Body::from_bytes(INTERNAL_SERVER_ERROR.into()));
            Ok(res)
        }
    }
}

async fn serve(req: Request) -> nephele::common::http_types::Result<Response> {
    match (req.method(), req.url().path()) {
        (Method::Get, "/") | (Method::Get, "/index.html") => {
            let mut res = Response::new(StatusCode::Ok);
            res.set_body(Body::from_bytes(INDEX_STR.into()));
            Ok(res)
        }
        (Method::Get, "/fetch.html") => api_fetch(req).await,
        (Method::Get, "/fetch_upstream") => api_fetch_upstream().await,
        (Method::Post, "/json_api") => api_post_json(req).await,
        (Method::Get, "/json_api") => api_get_json().await,
        _ => {
            let mut res = Response::new(StatusCode::NotFound);
            res.set_body(Body::from_bytes(NOTFOUND_STR.into()));
            Ok(res)
        }
    }
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

    let http = listen(Async::<TcpListener>::bind(([127, 0, 0, 1], 7000))?, None);
    let https = listen(
        Async::<TcpListener>::bind(([127, 0, 0, 1], 8002))?,
        Some(tls),
    );
    future::try_zip(http, https).await?;
    Ok(())
}
