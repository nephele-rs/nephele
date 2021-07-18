use cynthia::future::swap::{self, AsyncRead, Cursor};
use cynthia::runtime::task::{Context, Poll};
use std::io::Write;
use std::pin::Pin;

use crate::common::http_types::headers::{CONTENT_LENGTH, HOST, TRANSFER_ENCODING};
use crate::common::http_types::{Method, Request};
use crate::proto::h1::body_encoder::BodyEncoder;
use crate::proto::h1::EncoderState;
use crate::read_to_end;

#[doc(hidden)]
#[derive(Debug)]
pub struct Encoder {
    request: Request,
    state: EncoderState,
}

impl Encoder {
    pub fn new(request: Request) -> Self {
        Self {
            request,
            state: EncoderState::Start,
        }
    }

    fn finalize_headers(&mut self) -> swap::Result<()> {
        if self.request.header(HOST).is_none() {
            let url = self.request.url();
            let host = url
                .host_str()
                .ok_or_else(|| swap::Error::new(swap::ErrorKind::InvalidData, "Missing hostname"))?
                .to_owned();

            if let Some(port) = url.port() {
                self.request
                    .insert_header(HOST, format!("{}:{}", host, port));
            } else {
                self.request.insert_header(HOST, host);
            };
        }

        if self.request.method() == Method::Connect {
            self.request.insert_header("proxy-connection", "keep-alive");
        }

        if let Some(len) = self.request.len() {
            self.request.insert_header(CONTENT_LENGTH, len.to_string());
        } else {
            self.request.insert_header(TRANSFER_ENCODING, "chunked");
        }

        Ok(())
    }

    fn compute_head(&mut self) -> swap::Result<Cursor<Vec<u8>>> {
        let mut buf = Vec::with_capacity(128);
        let url = self.request.url();
        let method = self.request.method();
        write!(buf, "{} ", method)?;

        if method == Method::Connect {
            let host = url.host_str().ok_or_else(|| {
                swap::Error::new(swap::ErrorKind::InvalidData, "Missing hostname")
            })?;

            let port = url.port_or_known_default().ok_or_else(|| {
                swap::Error::new(
                    swap::ErrorKind::InvalidData,
                    "Unexpected scheme with no default port",
                )
            })?;

            write!(buf, "{}:{}", host, port)?;
        } else {
            write!(buf, "{}", url.path())?;
            if let Some(query) = url.query() {
                write!(buf, "?{}", query)?;
            }
        }

        write!(buf, " HTTP/1.1\r\n")?;

        self.finalize_headers()?;
        let mut headers = self.request.iter().collect::<Vec<_>>();
        headers.sort_unstable_by_key(|(h, _)| if **h == HOST { "0" } else { h.as_str() });
        for (header, values) in headers {
            for value in values.iter() {
                write!(buf, "{}: {}\r\n", header, value)?;
            }
        }

        write!(buf, "\r\n")?;
        Ok(Cursor::new(buf))
    }
}

impl AsyncRead for Encoder {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<swap::Result<usize>> {
        loop {
            self.state = match self.state {
                EncoderState::Start => EncoderState::Head(self.compute_head()?),

                EncoderState::Head(ref mut cursor) => {
                    read_to_end!(Pin::new(cursor).poll_read(cx, buf));
                    EncoderState::Body(BodyEncoder::new(self.request.take_body()))
                }

                EncoderState::Body(ref mut encoder) => {
                    read_to_end!(Pin::new(encoder).poll_read(cx, buf));
                    EncoderState::End
                }

                EncoderState::End => return Poll::Ready(Ok(0)),
            }
        }
    }
}
