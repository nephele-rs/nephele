use cynthia::future::prelude::*;
use cynthia::future::swap::{AsyncRead, BufferReader};
use std::convert::TryFrom;

use crate::{ensure, ensure_eq, format_err};

use crate::common::http_types::{
    headers::{CONTENT_LENGTH, DATE, TRANSFER_ENCODING},
    Body, Response, StatusCode,
};

use crate::proto::h1::chunked::ChunkedDecoder;
use crate::proto::h1::date::fmt_http_date;
use crate::proto::h1::{MAX_HEADERS, MAX_HEAD_LENGTH};

const CR: u8 = b'\r';
const LF: u8 = b'\n';

pub async fn decode<R>(reader: R) -> crate::common::http_types::Result<Response>
where
    R: AsyncRead + Unpin + Send + Sync + 'static,
{
    let mut reader = BufferReader::new(reader);
    let mut buf = Vec::new();
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut httparse_res = httparse::Response::new(&mut headers);

    loop {
        let bytes_read = reader.read_until(LF, &mut buf).await?;
        assert!(bytes_read != 0, "Empty response");

        ensure!(
            buf.len() < MAX_HEAD_LENGTH,
            "Head byte length should be less than 8kb"
        );

        let idx = buf.len() - 1;
        if idx >= 3 && buf[idx - 3..=idx] == [CR, LF, CR, LF] {
            break;
        }
        if idx >= 1 && buf[idx - 1..=idx] == [LF, LF] {
            break;
        }
    }

    let status = httparse_res.parse(&buf)?;
    ensure!(!status.is_partial(), "Malformed HTTP head");

    let code = httparse_res.code;
    let code = code.ok_or_else(|| format_err!("No status code found"))?;

    let version = httparse_res.version;
    let version = version.ok_or_else(|| format_err!("No version found"))?;
    ensure_eq!(version, 1, "Unsupported HTTP version");

    let mut res = Response::new(StatusCode::try_from(code)?);
    for header in httparse_res.headers.iter() {
        res.append_header(header.name, std::str::from_utf8(header.value)?);
    }

    if res.header(DATE).is_none() {
        let date = fmt_http_date(std::time::SystemTime::now());
        res.insert_header(DATE, &format!("date: {}\r\n", date)[..]);
    }

    let content_length = res.header(CONTENT_LENGTH);
    let transfer_encoding = res.header(TRANSFER_ENCODING);

    ensure!(
        content_length.is_none() || transfer_encoding.is_none(),
        "Unexpected Content-Length header"
    );

    if let Some(encoding) = transfer_encoding {
        if encoding.last().as_str() == "chunked" {
            let trailers_sender = res.send_trailers();
            let reader = BufferReader::new(ChunkedDecoder::new(reader, trailers_sender));
            res.set_body(Body::from_reader(reader, None));

            return Ok(res);
        }
    }

    if let Some(len) = content_length {
        let len = len.last().as_str().parse::<usize>()?;
        res.set_body(Body::from_reader(reader.take(len as u64), Some(len)));
    }

    Ok(res)
}
