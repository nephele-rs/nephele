use anyhow::Result;
use http::{HeaderMap, Request};

use cynthia::runtime::{self, transport};

use nephele::proto::h2::client;

fn main() -> Result<()> {
    cynthia::runtime::block_on(async {
        let tcp = transport::TcpStream::connect("127.0.0.1:7000").await?;
        let (mut client, h2) = client::handshake(tcp).await?;

        let request = Request::builder()
            .uri("http://127.0.0.1:7000")
            .body(())
            .unwrap();

        let mut trailers = HeaderMap::new();
        trailers.insert("zomg", "hello".parse().unwrap());

        let (response, mut stream) = client.send_request(request, false).unwrap();
        stream.send_trailers(trailers).unwrap();

        runtime::spawn(async move {
            if let Err(e) = h2.await {
                println!("GOT ERR={:?}", e);
            }
        })
        .detach();

        let response = response.await?;
        let mut body = response.into_body();

        while let Some(chunk) = body.data().await {
            println!("GOT CHUNK = {:?}", chunk?);
        }

        if let Some(trailers) = body.trailers().await? {
            println!("GOT TRAILERS: {:?}", trailers);
        }

        Ok(())
    })
}
