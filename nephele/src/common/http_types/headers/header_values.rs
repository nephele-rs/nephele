use std::fmt::{self, Debug, Display};
use std::iter::FromIterator;
use std::ops::{Deref, DerefMut, Index};
use std::slice::SliceIndex;

use crate::common::http_types::headers::{HeaderValue, Values};

#[derive(Clone)]
pub struct HeaderValues {
    pub(crate) inner: Vec<HeaderValue>,
}

impl HeaderValues {
    pub fn append(&mut self, other: &mut Self) {
        self.inner.append(&mut other.inner)
    }

    pub fn get(&self, index: usize) -> Option<&HeaderValue> {
        self.inner.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut HeaderValue> {
        self.inner.get_mut(index)
    }

    pub fn contains(&self, value: &HeaderValue) -> bool {
        self.inner.contains(value)
    }

    pub fn last(&self) -> &HeaderValue {
        self.inner
            .last()
            .expect("HeaderValues must always contain at least one value")
    }

    pub fn iter(&self) -> Values<'_> {
        Values::new_values(&self)
    }
}

impl<I: SliceIndex<[HeaderValue]>> Index<I> for HeaderValues {
    type Output = I::Output;

    #[inline]
    fn index(&self, index: I) -> &Self::Output {
        Index::index(&self.inner, index)
    }
}

impl FromIterator<HeaderValue> for HeaderValues {
    fn from_iter<I>(iter: I) -> HeaderValues
    where
        I: IntoIterator<Item = HeaderValue>,
    {
        let iter = iter.into_iter();
        let mut output = Vec::with_capacity(iter.size_hint().0);
        for v in iter {
            output.push(v);
        }
        HeaderValues { inner: output }
    }
}

impl Debug for HeaderValues {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.inner.len() == 1 {
            write!(f, "{:?}", self.inner[0])
        } else {
            f.debug_list().entries(self.inner.iter()).finish()
        }
    }
}

impl Display for HeaderValues {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut list = f.debug_list();
        for v in &self.inner {
            list.entry(&v);
        }
        list.finish()
    }
}

impl PartialEq<str> for HeaderValues {
    fn eq(&self, other: &str) -> bool {
        self.inner.len() == 1 && self.inner[0] == other
    }
}

impl<'a> PartialEq<&'a str> for HeaderValues {
    fn eq(&self, other: &&'a str) -> bool {
        self.inner.len() == 1 && &self.inner[0] == other
    }
}

impl<'a> PartialEq<[&'a str]> for HeaderValues {
    fn eq(&self, other: &[&'a str]) -> bool {
        self.inner.iter().eq(other.iter())
    }
}

impl PartialEq<String> for HeaderValues {
    fn eq(&self, other: &String) -> bool {
        self.inner.len() == 1 && self.inner[0] == *other
    }
}

impl<'a> PartialEq<&String> for HeaderValues {
    fn eq(&self, other: &&String) -> bool {
        self.inner.len() == 1 && self.inner[0] == **other
    }
}

impl From<HeaderValue> for HeaderValues {
    fn from(other: HeaderValue) -> Self {
        Self { inner: vec![other] }
    }
}

impl AsRef<HeaderValue> for HeaderValues {
    fn as_ref(&self) -> &HeaderValue {
        &self.inner[0]
    }
}

impl AsMut<HeaderValue> for HeaderValues {
    fn as_mut(&mut self) -> &mut HeaderValue {
        &mut self.inner[0]
    }
}
impl Deref for HeaderValues {
    type Target = HeaderValue;

    fn deref(&self) -> &HeaderValue {
        &self.inner[0]
    }
}

impl DerefMut for HeaderValues {
    fn deref_mut(&mut self) -> &mut HeaderValue {
        &mut self.inner[0]
    }
}

impl<'a> IntoIterator for &'a HeaderValues {
    type Item = &'a HeaderValue;
    type IntoIter = Values<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
