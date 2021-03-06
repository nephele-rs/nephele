use std::collections::hash_map;
use std::iter::Iterator;

use crate::common::http_types::headers::{HeaderName, HeaderValues};

#[derive(Debug)]
pub struct IntoIter {
    pub(super) inner: hash_map::IntoIter<HeaderName, HeaderValues>,
}

impl Iterator for IntoIter {
    type Item = (HeaderName, HeaderValues);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}
