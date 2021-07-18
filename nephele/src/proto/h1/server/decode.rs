use cynthia::future::prelude::*;
use cynthia::future::swap::{AsyncRead, AsyncWrite, BufferReader};
use cynthia::platform::dup::{Arc, Mutex};
use std::str::FromStr;

use crate::common::http_types::content::ContentLength;
use crate::common::http_types::headers::{EXPECT, TRANSFER_ENCODING};
use crate::common::http_types::{Body, Method, Request, Url};
use crate::{ensure, ensure_eq, format_err};

use crate::proto::h1::chunked::ChunkedDecoder;
use crate::proto::h1::read_notifier::ReadNotifier;
use crate::proto::h1::server::body_reader::BodyReader;
use crate::proto::h1::{MAX_HEADERS, MAX_HEAD_LENGTH};

const LF: u8 = b'\n';

const HTTP_1_1_VERSION: u8 = 1;

const CONTINUE_HEADER_VALUE: &str = "100-continue";
const CONTINUE_RESPONSE: &[u8] = b"HTTP/1.1 100 Continue\r\n\r\n";

pub async fn decode<IO>(
    mut io: IO,
) -> crate::common::http_types::Result<Option<(Request, BodyReader<IO>)>>
where
    IO: AsyncRead + AsyncWrite + Clone + Send + Sync + Unpin + 'static,
{
    let mut reader = BufferReader::new(io.clone());
    let mut buf = Vec::new();
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];

    let mut httparse_req = httparse::Request::new(&mut headers);

    println!("headers = {:?}", httparse_req);

    loop {
        let bytes_read = reader.read_until(LF, &mut buf).await?;
        if bytes_read == 0 {
            return Ok(None);
        }

        ensure!(
            buf.len() < MAX_HEAD_LENGTH,
            "Head byte length should be less than 8kb"
        );

        let idx = buf.len() - 1;
        if idx >= 3 && &buf[idx - 3..=idx] == b"\r\n\r\n" {
            println!("brrrrrrr {}", idx);
            break;
        }
    }

    println!("version = {:?}", buf);

    let status = httparse_req.parse(&buf)?;

    ensure!(!status.is_partial(), "Malformed HTTP head");

    let method = httparse_req.method;
    let method = method.ok_or_else(|| format_err!("No method found"))?;

    let version = httparse_req.version;
    println!("version = {:?}", version);
    let version = version.ok_or_else(|| format_err!("No version found"))?;

    println!("version = {} {}", method, version);

    ensure_eq!(
        version,
        HTTP_1_1_VERSION,
        "Unsupported HTTP version 1.{}",
        version
    );

    println!("111 version = {} {}", method, version);

    let url = url_from_httparse_req(&httparse_req)?;

    let mut req = Request::new(Method::from_str(method)?, url);

    req.set_version(Some(crate::common::http_types::Version::Http1_1));

    for header in httparse_req.headers.iter() {
        req.append_header(header.name, std::str::from_utf8(header.value)?);
    }

    let content_length = ContentLength::from_headers(&req)?;
    let transfer_encoding = req.header(TRANSFER_ENCODING);

    crate::ensure_status!(
        content_length.is_none() || transfer_encoding.is_none(),
        400,
        "Unexpected Content-Length header"
    );

    let (body_read_sender, body_read_receiver) = cynthia::platform::channel::bounded(1);

    if Some(CONTINUE_HEADER_VALUE) == req.header(EXPECT).map(|h| h.as_str()) {
        cynthia::runtime::spawn(async move {
            if let Ok(()) = body_read_receiver.recv().await {
                io.write_all(CONTINUE_RESPONSE).await.ok();
            };
        })
        .detach();
    }

    if transfer_encoding
        .map(|te| te.as_str().eq_ignore_ascii_case("chunked"))
        .unwrap_or(false)
    {
        let trailer_sender = req.send_trailers();
        let reader = ChunkedDecoder::new(reader, trailer_sender);
        let reader = Arc::new(Mutex::new(reader));
        let reader_clone = reader.clone();
        let reader = ReadNotifier::new(reader, body_read_sender);
        let reader = BufferReader::new(reader);
        req.set_body(Body::from_reader(reader, None));
        return Ok(Some((req, BodyReader::Chunked(reader_clone))));
    } else if let Some(len) = content_length {
        let len = len.len();
        let reader = Arc::new(Mutex::new(reader.take(len)));
        req.set_body(Body::from_reader(
            BufferReader::new(ReadNotifier::new(reader.clone(), body_read_sender)),
            Some(len as usize),
        ));
        Ok(Some((req, BodyReader::Fixed(reader))))
    } else {
        Ok(Some((req, BodyReader::None)))
    }
}

fn url_from_httparse_req(
    req: &httparse::Request<'_, '_>,
) -> crate::common::http_types::Result<Url> {
    let path = req.path.ok_or_else(|| format_err!("No uri found"))?;

    let host = req
        .headers
        .iter()
        .find(|x| x.name.eq_ignore_ascii_case("host"))
        .ok_or_else(|| format_err!("Mandatory Host header missing"))?
        .value;

    let host = std::str::from_utf8(host)?;

    if path.starts_with("http://") || path.starts_with("https://") {
        Ok(Url::parse(path)?)
    } else if path.starts_with('/') {
        Ok(Url::parse(&format!("http://{}{}", host, path))?)
    } else if req.method.unwrap().eq_ignore_ascii_case("connect") {
        Ok(Url::parse(&format!("http://{}/", path))?)
    } else {
        Err(crate::format_err!("unexpected uri format"))
    }
}
