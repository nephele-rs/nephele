use cynthia::future::swap::{self, AsyncRead, AsyncWrite};

use crate::common::http_types::{Request, Response};

mod decode;
mod encode;

pub use decode::decode;
pub use encode::Encoder;

pub async fn connect<RW>(
    mut stream: RW,
    req: Request,
) -> crate::common::http_types::Result<Response>
where
    RW: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static,
{
    let mut req = Encoder::new(req);

    swap::copy(&mut req, &mut stream).await?;

    let res = decode(stream).await?;

    Ok(res)
}
