use crate::common::http_types::headers::{HeaderName, HeaderValue, Headers, CONTENT_LENGTH};
use crate::common::http_types::Status;

#[derive(Debug)]
pub struct ContentLength {
    length: u64,
}

#[allow(clippy::len_without_is_empty)]
impl ContentLength {
    pub fn new(length: u64) -> Self {
        Self { length }
    }

    pub fn from_headers(
        headers: impl AsRef<Headers>,
    ) -> crate::common::http_types::Result<Option<Self>> {
        let headers = match headers.as_ref().get(CONTENT_LENGTH) {
            Some(headers) => headers,
            None => return Ok(None),
        };

        let value = headers.iter().last().unwrap();
        let length = value.as_str().trim().parse().status(400)?;
        Ok(Some(Self { length }))
    }

    pub fn apply(&self, mut headers: impl AsMut<Headers>) {
        headers.as_mut().insert(self.name(), self.value());
    }

    pub fn name(&self) -> HeaderName {
        CONTENT_LENGTH
    }

    pub fn value(&self) -> HeaderValue {
        let output = format!("{}", self.length);

        unsafe { HeaderValue::from_bytes_unchecked(output.into()) }
    }

    pub fn len(&self) -> u64 {
        self.length
    }

    pub fn set_len(&mut self, len: u64) {
        self.length = len;
    }
}
