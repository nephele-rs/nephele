use std::collections::hash_map;
use std::iter::Iterator;

use crate::common::http_types::headers::{HeaderName, HeaderValue, HeaderValues};

#[derive(Debug)]
pub struct Values<'a> {
    pub(super) inner: Option<hash_map::Values<'a, HeaderName, HeaderValues>>,
    slot: Option<&'a HeaderValues>,
    cursor: usize,
}

impl<'a> Values<'a> {
    pub(crate) fn new(inner: hash_map::Values<'a, HeaderName, HeaderValues>) -> Self {
        Self {
            inner: Some(inner),
            slot: None,
            cursor: 0,
        }
    }

    pub(crate) fn new_values(values: &'a HeaderValues) -> Self {
        Self {
            inner: None,
            slot: Some(values),
            cursor: 0,
        }
    }
}

impl<'a> Iterator for Values<'a> {
    type Item = &'a HeaderValue;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.slot.is_none() {
                let next = match self.inner.as_mut() {
                    Some(inner) => inner.next()?,
                    None => return None,
                };
                self.cursor = 0;
                self.slot = Some(next);
            }

            match self.slot.unwrap().get(self.cursor) {
                Some(item) => {
                    self.cursor += 1;
                    return Some(item);
                }
                None => {
                    self.slot = None;
                    continue;
                }
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.inner.as_ref() {
            Some(inner) => inner.size_hint(),
            None => (0, None),
        }
    }
}
