use cynthia::future::{prelude::*, swap};
use cynthia::ready;
use serde::{de::DeserializeOwned, Serialize};
use std::fmt::{self, Debug};
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::common::http_types::{mime, Mime};
use crate::common::http_types::{Status, StatusCode};

pin_project_lite::pin_project! {
    pub struct Body {
        #[pin]
        reader: Box<dyn AsyncBufRead + Unpin + Send + Sync + 'static>,
        mime: Mime,
        length: Option<usize>,
        bytes_read: usize
    }
}

impl Body {
    pub fn empty() -> Self {
        Self {
            reader: Box::new(swap::empty()),
            mime: mime::BYTE_STREAM,
            length: Some(0),
            bytes_read: 0,
        }
    }

    pub fn from_reader(
        reader: impl AsyncBufRead + Unpin + Send + Sync + 'static,
        len: Option<usize>,
    ) -> Self {
        Self {
            reader: Box::new(reader),
            mime: mime::BYTE_STREAM,
            length: len,
            bytes_read: 0,
        }
    }

    pub fn into_reader(self) -> Box<dyn AsyncBufRead + Unpin + Send + Sync + 'static> {
        self.reader
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            mime: mime::BYTE_STREAM,
            length: Some(bytes.len()),
            reader: Box::new(swap::Cursor::new(bytes)),
            bytes_read: 0,
        }
    }

    pub async fn into_bytes(mut self) -> crate::common::http_types::Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(1024);
        self.read_to_end(&mut buf)
            .await
            .status(StatusCode::UnprocessableEntity)?;
        Ok(buf)
    }

    pub fn from_string(s: String) -> Self {
        Self {
            mime: mime::PLAIN,
            length: Some(s.len()),
            reader: Box::new(swap::Cursor::new(s.into_bytes())),
            bytes_read: 0,
        }
    }

    pub async fn into_string(mut self) -> crate::common::http_types::Result<String> {
        let mut result = String::with_capacity(self.len().unwrap_or(0));
        self.read_to_string(&mut result)
            .await
            .status(StatusCode::UnprocessableEntity)?;
        Ok(result)
    }

    pub fn from_json(json: &impl Serialize) -> crate::common::http_types::Result<Self> {
        let bytes = serde_json::to_vec(&json)?;
        let body = Self {
            length: Some(bytes.len()),
            reader: Box::new(swap::Cursor::new(bytes)),
            mime: mime::JSON,
            bytes_read: 0,
        };
        Ok(body)
    }

    pub async fn into_json<T: DeserializeOwned>(mut self) -> crate::common::http_types::Result<T> {
        let mut buf = Vec::with_capacity(1024);
        self.read_to_end(&mut buf).await?;
        Ok(serde_json::from_slice(&buf).status(StatusCode::UnprocessableEntity)?)
    }

    pub fn from_form(form: &impl Serialize) -> crate::common::http_types::Result<Self> {
        let query = serde_urlencoded::to_string(form)?;
        let bytes = query.into_bytes();

        let body = Self {
            length: Some(bytes.len()),
            reader: Box::new(swap::Cursor::new(bytes)),
            mime: mime::FORM,
            bytes_read: 0,
        };
        Ok(body)
    }

    pub async fn into_form<T: DeserializeOwned>(self) -> crate::common::http_types::Result<T> {
        let s = self.into_string().await?;
        Ok(serde_urlencoded::from_str(&s).status(StatusCode::UnprocessableEntity)?)
    }

    #[cfg(all(feature = "fs", not(target_os = "unknown")))]
    pub async fn from_file<P>(path: P) -> io::Result<Self>
    where
        P: AsRef<std::path::Path>,
    {
        let path = path.as_ref();
        let mut file = file::File::open(path).await?;
        let len = file.metadata().await?.len();

        let mime = peek_mime(&mut file)
            .await?
            .or_else(|| guess_ext(path))
            .unwrap_or(mime::BYTE_STREAM);

        Ok(Self {
            mime,
            length: Some(len as usize),
            reader: Box::new(io::BufferReader::new(file)),
            bytes_read: 0,
        })
    }

    pub fn len(&self) -> Option<usize> {
        self.length
    }

    pub fn is_empty(&self) -> Option<bool> {
        self.length.map(|length| length == 0)
    }

    pub fn mime(&self) -> &Mime {
        &self.mime
    }

    pub fn set_mime(&mut self, mime: impl Into<Mime>) {
        self.mime = mime.into();
    }
}

impl Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Body")
            .field("reader", &"<hidden>")
            .field("length", &self.length)
            .field("bytes_read", &self.bytes_read)
            .finish()
    }
}

impl From<serde_json::Value> for Body {
    fn from(json_value: serde_json::Value) -> Self {
        Self::from_json(&json_value).unwrap()
    }
}

impl From<String> for Body {
    fn from(s: String) -> Self {
        Self::from_string(s)
    }
}

impl<'a> From<&'a str> for Body {
    fn from(s: &'a str) -> Self {
        Self::from_string(s.to_owned())
    }
}

impl From<Vec<u8>> for Body {
    fn from(b: Vec<u8>) -> Self {
        Self::from_bytes(b)
    }
}

impl<'a> From<&'a [u8]> for Body {
    fn from(b: &'a [u8]) -> Self {
        Self::from_bytes(b.to_owned())
    }
}

impl AsyncRead for Body {
    #[allow(missing_doc_code_examples)]
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<swap::Result<usize>> {
        let mut buf = match self.length {
            None => buf,
            Some(length) if length == self.bytes_read => return Poll::Ready(Ok(0)),
            Some(length) => {
                let max_len = (length - self.bytes_read).min(buf.len());
                &mut buf[0..max_len]
            }
        };

        let bytes = ready!(Pin::new(&mut self.reader).poll_read(cx, &mut buf))?;
        self.bytes_read += bytes;
        Poll::Ready(Ok(bytes))
    }
}

impl AsyncBufRead for Body {
    #[allow(missing_doc_code_examples)]
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<swap::Result<&'_ [u8]>> {
        self.project().reader.poll_fill_buf(cx)
    }

    fn consume(mut self: Pin<&mut Self>, amt: usize) {
        Pin::new(&mut self.reader).consume(amt)
    }
}

#[cfg(all(feature = "fs", not(target_os = "unknown")))]
async fn peek_mime(file: &mut file::File) -> io::Result<Option<Mime>> {
    let mut buf = [0_u8; 300];
    file.read(&mut buf).await?;
    let mime = Mime::sniff(&buf).ok();

    file.seek(io::SeekFrom::Start(0)).await?;
    Ok(mime)
}

#[cfg(all(feature = "fs", not(target_os = "unknown")))]
fn guess_ext(path: &std::path::Path) -> Option<Mime> {
    let ext = path.extension().map(|p| p.to_str()).flatten();
    ext.and_then(Mime::from_extension)
}
