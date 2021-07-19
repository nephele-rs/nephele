use bytes::Bytes;
use std::error::Error;
use std::net::{TcpListener, TcpStream};

use cynthia::runtime::{self, swap, Async};
use nephele::proto::h2::server;

async fn handle(socket: Async<TcpStream>) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut connection = server::handshake(socket).await?;

    while let Some(result) = connection.accept().await {
        let (_request, mut respond) = result?;
        let response = http::Response::new(());

        let mut send = respond.send_response(response, false)?;
        send.send_data(Bytes::from_static(b"hello world, h2 is me"), true)?;
    }

    Ok(())
}

#[cynthia::main]
async fn main() -> swap::Result<()> {
    let listener = Async::<TcpListener>::bind(([0, 0, 0, 0], 7000))?;
    loop {
        let (stream, _peer_addr) = listener.accept().await?;

        runtime::spawn(handle(stream)).detach();
    }
}
