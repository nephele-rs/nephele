use std::convert::TryFrom;
use std::fmt::{self, Debug, Display};
use std::str::FromStr;

use crate::common::http_types::headers::HeaderValues;
use crate::common::http_types::Error;
use crate::common::http_types::Mime;
#[cfg(feature = "cookies")]
use crate::Cookie;

#[derive(Clone, Eq, PartialEq, Hash)]
pub struct HeaderValue {
    inner: String,
}

impl HeaderValue {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        crate::ensure!(bytes.is_ascii(), "Bytes should be valid ASCII");

        let string = unsafe { String::from_utf8_unchecked(bytes) };
        Ok(Self { inner: string })
    }

    pub unsafe fn from_bytes_unchecked(bytes: Vec<u8>) -> Self {
        let string = String::from_utf8_unchecked(bytes);
        Self { inner: string }
    }

    pub fn as_str(&self) -> &str {
        &self.inner
    }
}

impl From<Mime> for HeaderValue {
    fn from(mime: Mime) -> Self {
        HeaderValue {
            inner: format!("{}", mime),
        }
    }
}

#[cfg(feature = "cookies")]
impl From<Cookie<'_>> for HeaderValue {
    fn from(cookie: Cookie<'_>) -> Self {
        HeaderValue {
            inner: cookie.to_string(),
        }
    }
}

impl From<&Mime> for HeaderValue {
    fn from(mime: &Mime) -> Self {
        HeaderValue {
            inner: format!("{}", mime),
        }
    }
}

impl FromStr for HeaderValue {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        crate::ensure!(s.is_ascii(), "String slice should be valid ASCII");
        Ok(Self {
            inner: String::from(s),
        })
    }
}

impl<'a> TryFrom<&'a str> for HeaderValue {
    type Error = Error;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

impl Debug for HeaderValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.inner)
    }
}

impl Display for HeaderValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner)
    }
}

impl PartialEq<str> for HeaderValue {
    fn eq(&self, other: &str) -> bool {
        self.inner == other
    }
}

impl<'a> PartialEq<&'a str> for HeaderValue {
    fn eq(&self, other: &&'a str) -> bool {
        &self.inner == other
    }
}

impl PartialEq<String> for HeaderValue {
    fn eq(&self, other: &String) -> bool {
        &self.inner == other
    }
}

impl<'a> PartialEq<&String> for HeaderValue {
    fn eq(&self, other: &&String) -> bool {
        &&self.inner == other
    }
}

impl From<HeaderValues> for HeaderValue {
    fn from(mut other: HeaderValues) -> Self {
        other.inner.reverse();
        other
            .inner
            .pop()
            .expect("HeaderValues should contain at least one value")
    }
}
