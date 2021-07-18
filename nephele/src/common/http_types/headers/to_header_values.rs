use std::borrow::Cow;
use std::{io, iter, option, slice};

use crate::common::http_types::headers::{HeaderValue, HeaderValues, Values};

pub trait ToHeaderValues {
    type Iter: Iterator<Item = HeaderValue>;

    fn to_header_values(&self) -> crate::common::http_types::Result<Self::Iter>;
}

impl ToHeaderValues for HeaderValue {
    type Iter = option::IntoIter<HeaderValue>;

    fn to_header_values(&self) -> crate::common::http_types::Result<Self::Iter> {
        Ok(Some(self.clone()).into_iter())
    }
}

impl<'a> ToHeaderValues for &'a HeaderValues {
    type Iter = iter::Cloned<Values<'a>>;

    fn to_header_values(&self) -> crate::common::http_types::Result<Self::Iter> {
        Ok(self.iter().cloned())
    }
}

impl<'a> ToHeaderValues for &'a [HeaderValue] {
    type Iter = iter::Cloned<slice::Iter<'a, HeaderValue>>;

    fn to_header_values(&self) -> crate::common::http_types::Result<Self::Iter> {
        Ok(self.iter().cloned())
    }
}

impl<'a> ToHeaderValues for &'a str {
    type Iter = option::IntoIter<HeaderValue>;

    fn to_header_values(&self) -> crate::common::http_types::Result<Self::Iter> {
        let value = self
            .parse()
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
        Ok(Some(value).into_iter())
    }
}

impl ToHeaderValues for String {
    type Iter = option::IntoIter<HeaderValue>;

    fn to_header_values(&self) -> crate::common::http_types::Result<Self::Iter> {
        self.as_str().to_header_values()
    }
}

impl ToHeaderValues for &String {
    type Iter = option::IntoIter<HeaderValue>;

    fn to_header_values(&self) -> crate::common::http_types::Result<Self::Iter> {
        self.as_str().to_header_values()
    }
}

impl ToHeaderValues for Cow<'_, str> {
    type Iter = option::IntoIter<HeaderValue>;

    fn to_header_values(&self) -> crate::common::http_types::Result<Self::Iter> {
        self.as_ref().to_header_values()
    }
}
