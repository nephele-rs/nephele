use std::borrow::Cow;
use std::fmt::{self, Debug, Display};
use std::str::FromStr;

use crate::common::http_types::Error;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct HeaderName(Cow<'static, str>);

impl HeaderName {
    pub fn from_bytes(mut bytes: Vec<u8>) -> Result<Self, Error> {
        crate::ensure!(bytes.is_ascii(), "Bytes should be valid ASCII");
        bytes.make_ascii_lowercase();

        let string = unsafe { String::from_utf8_unchecked(bytes.to_vec()) };
        Ok(HeaderName(Cow::Owned(string)))
    }

    pub fn from_string(s: String) -> Result<Self, Error> {
        Self::from_bytes(s.into_bytes())
    }

    pub fn as_str(&self) -> &'_ str {
        &self.0
    }

    pub unsafe fn from_bytes_unchecked(mut bytes: Vec<u8>) -> Self {
        bytes.make_ascii_lowercase();
        let string = String::from_utf8_unchecked(bytes);
        HeaderName(Cow::Owned(string))
    }

    pub(crate) const fn from_lowercase_str(str: &'static str) -> Self {
        HeaderName(Cow::Borrowed(str))
    }
}

impl Debug for HeaderName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl Display for HeaderName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for HeaderName {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        crate::ensure!(s.is_ascii(), "String slice should be valid ASCII");
        Ok(HeaderName(Cow::Owned(s.to_ascii_lowercase())))
    }
}

impl From<&HeaderName> for HeaderName {
    fn from(value: &HeaderName) -> HeaderName {
        value.clone()
    }
}

impl<'a> From<&'a str> for HeaderName {
    fn from(value: &'a str) -> Self {
        Self::from_str(value).expect("String slice should be valid ASCII")
    }
}

impl PartialEq<str> for HeaderName {
    fn eq(&self, other: &str) -> bool {
        match HeaderName::from_str(other) {
            Err(_) => false,
            Ok(other) => self == &other,
        }
    }
}

impl<'a> PartialEq<&'a str> for HeaderName {
    fn eq(&self, other: &&'a str) -> bool {
        match HeaderName::from_str(other) {
            Err(_) => false,
            Ok(other) => self == &other,
        }
    }
}

impl PartialEq<String> for HeaderName {
    fn eq(&self, other: &String) -> bool {
        match HeaderName::from_str(&other) {
            Err(_) => false,
            Ok(other) => self == &other,
        }
    }
}

impl<'a> PartialEq<&String> for HeaderName {
    fn eq(&self, other: &&String) -> bool {
        match HeaderName::from_str(other) {
            Err(_) => false,
            Ok(other) => self == &other,
        }
    }
}
