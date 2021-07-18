use std::net::{TcpListener, TcpStream};

use cynthia::future::prelude::*;
use cynthia::runtime::{self, swap, Async};

async fn echo(mut stream: Async<TcpStream>) -> swap::Result<()> {
    let mut buf = vec![0u8; 1024];

    loop {
        let n = stream.read(&mut buf).await?;

        if n == 0 {
            return Ok(());
        }

        stream.write_all(&buf[0..n]).await?;
    }
}

fn main() -> swap::Result<()> {
    runtime::block_on(async {
        let listener = Async::<TcpListener>::bind(([127, 0, 0, 1], 7000))?;

        loop {
            let (stream, _peer_addr) = listener.accept().await?;
            runtime::spawn(echo(stream)).detach();
        }
    })
}
