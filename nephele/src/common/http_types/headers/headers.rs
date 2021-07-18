use std::collections::HashMap;
use std::convert::Into;
use std::fmt::{self, Debug};
use std::iter::IntoIterator;
use std::ops::Index;
use std::str::FromStr;

use crate::common::http_types::headers::{
    HeaderName, HeaderValues, IntoIter, Iter, IterMut, Names, ToHeaderValues, Values,
};

#[derive(Clone)]
pub struct Headers {
    pub(crate) headers: HashMap<HeaderName, HeaderValues>,
}

impl Headers {
    pub(crate) fn new() -> Self {
        Self {
            headers: HashMap::new(),
        }
    }

    pub fn insert(
        &mut self,
        name: impl Into<HeaderName>,
        values: impl ToHeaderValues,
    ) -> Option<HeaderValues> {
        let name = name.into();
        let values: HeaderValues = values.to_header_values().unwrap().collect();
        self.headers.insert(name, values)
    }

    pub fn append(&mut self, name: impl Into<HeaderName>, values: impl ToHeaderValues) {
        let name = name.into();
        match self.get_mut(&name) {
            Some(headers) => {
                let mut values: HeaderValues = values.to_header_values().unwrap().collect();
                headers.append(&mut values);
            }
            None => {
                self.insert(name, values);
            }
        }
    }

    pub fn get(&self, name: impl Into<HeaderName>) -> Option<&HeaderValues> {
        self.headers.get(&name.into())
    }

    pub fn get_mut(&mut self, name: impl Into<HeaderName>) -> Option<&mut HeaderValues> {
        self.headers.get_mut(&name.into())
    }

    pub fn remove(&mut self, name: impl Into<HeaderName>) -> Option<HeaderValues> {
        self.headers.remove(&name.into())
    }

    pub fn iter(&self) -> Iter<'_> {
        Iter {
            inner: self.headers.iter(),
        }
    }

    pub fn iter_mut(&mut self) -> IterMut<'_> {
        IterMut {
            inner: self.headers.iter_mut(),
        }
    }

    pub fn names(&self) -> Names<'_> {
        Names {
            inner: self.headers.keys(),
        }
    }

    pub fn values(&self) -> Values<'_> {
        Values::new(self.headers.values())
    }
}

impl Index<HeaderName> for Headers {
    type Output = HeaderValues;

    #[inline]
    fn index(&self, name: HeaderName) -> &HeaderValues {
        self.get(name).expect("no entry found for name")
    }
}

impl Index<&str> for Headers {
    type Output = HeaderValues;

    #[inline]
    fn index(&self, name: &str) -> &HeaderValues {
        let name = HeaderName::from_str(name).expect("string slice needs to be valid ASCII");
        self.get(name).expect("no entry found for name")
    }
}

impl IntoIterator for Headers {
    type Item = (HeaderName, HeaderValues);
    type IntoIter = IntoIter;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            inner: self.headers.into_iter(),
        }
    }
}

impl<'a> IntoIterator for &'a Headers {
    type Item = (&'a HeaderName, &'a HeaderValues);
    type IntoIter = Iter<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a mut Headers {
    type Item = (&'a HeaderName, &'a mut HeaderValues);
    type IntoIter = IterMut<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl Debug for Headers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.headers.iter()).finish()
    }
}

impl AsRef<Headers> for Headers {
    fn as_ref(&self) -> &Headers {
        self
    }
}

impl AsMut<Headers> for Headers {
    fn as_mut(&mut self) -> &mut Headers {
        self
    }
}
