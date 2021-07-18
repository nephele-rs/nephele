use cynthia::future::swap::{self, AsyncRead, Cursor};
use cynthia::runtime::task::{Context, Poll};
use std::io::Write;
use std::pin::Pin;
use std::time::SystemTime;

use crate::common::http_types::headers::{CONTENT_LENGTH, DATE, TRANSFER_ENCODING};
use crate::common::http_types::{Method, Response};
use crate::proto::h1::body_encoder::BodyEncoder;
use crate::proto::h1::date::fmt_http_date;
use crate::proto::h1::EncoderState;
use crate::read_to_end;

#[derive(Debug)]
pub struct Encoder {
    response: Response,
    state: EncoderState,
    method: Method,
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

                    if self.method == Method::Head {
                        EncoderState::End
                    } else {
                        EncoderState::Body(BodyEncoder::new(self.response.take_body()))
                    }
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

impl Encoder {
    pub fn new(response: Response, method: Method) -> Self {
        Self {
            method,
            response,
            state: EncoderState::Start,
        }
    }

    fn finalize_headers(&mut self) {
        if let Some(len) = self.response.len() {
            self.response.insert_header(CONTENT_LENGTH, len.to_string());
        } else {
            self.response.insert_header(TRANSFER_ENCODING, "chunked");
        }

        if self.response.header(DATE).is_none() {
            let date = fmt_http_date(SystemTime::now());
            self.response.insert_header(DATE, date);
        }
    }

    fn compute_head(&mut self) -> swap::Result<Cursor<Vec<u8>>> {
        let mut head = Vec::with_capacity(128);
        let reason = self.response.status().canonical_reason();
        let status = self.response.status();
        write!(head, "HTTP/1.1 {} {}\r\n", status, reason)?;

        self.finalize_headers();
        let mut headers = self.response.iter().collect::<Vec<_>>();
        headers.sort_unstable_by_key(|(h, _)| h.as_str());
        for (header, values) in headers {
            for value in values.iter() {
                write!(head, "{}: {}\r\n", header, value)?;
            }
        }
        write!(head, "\r\n")?;
        Ok(Cursor::new(head))
    }
}
