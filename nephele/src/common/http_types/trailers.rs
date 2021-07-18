use cynthia::future::Stream;
use std::convert::Into;
use std::future::Future;
use std::ops::{Deref, DerefMut, Index};
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::common::http_types::headers::{
    HeaderName, HeaderValues, Headers, Iter, IterMut, Names, ToHeaderValues, Values,
};

#[derive(Debug)]
pub struct Trailers {
    headers: Headers,
}

impl Trailers {
    pub fn new() -> Self {
        Self {
            headers: Headers::new(),
        }
    }

    pub fn insert(
        &mut self,
        name: impl Into<HeaderName>,
        values: impl ToHeaderValues,
    ) -> Option<HeaderValues> {
        self.headers.insert(name, values)
    }

    pub fn append(&mut self, name: impl Into<HeaderName>, values: impl ToHeaderValues) {
        self.headers.append(name, values)
    }

    pub fn get(&self, name: impl Into<HeaderName>) -> Option<&HeaderValues> {
        self.headers.get(name)
    }

    pub fn get_mut(&mut self, name: impl Into<HeaderName>) -> Option<&mut HeaderValues> {
        self.headers.get_mut(name)
    }

    pub fn remove(&mut self, name: impl Into<HeaderName>) -> Option<HeaderValues> {
        self.headers.remove(name)
    }

    pub fn iter(&self) -> Iter<'_> {
        self.headers.iter()
    }

    pub fn iter_mut(&mut self) -> IterMut<'_> {
        self.headers.iter_mut()
    }

    pub fn names(&self) -> Names<'_> {
        self.headers.names()
    }

    pub fn values(&self) -> Values<'_> {
        self.headers.values()
    }
}

impl Clone for Trailers {
    fn clone(&self) -> Self {
        Self {
            headers: Headers {
                headers: self.headers.headers.clone(),
            },
        }
    }
}

impl Deref for Trailers {
    type Target = Headers;

    fn deref(&self) -> &Self::Target {
        &self.headers
    }
}

impl DerefMut for Trailers {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.headers
    }
}

impl Index<HeaderName> for Trailers {
    type Output = HeaderValues;

    #[inline]
    fn index(&self, name: HeaderName) -> &HeaderValues {
        self.headers.index(name)
    }
}

impl Index<&str> for Trailers {
    type Output = HeaderValues;

    #[inline]
    fn index(&self, name: &str) -> &HeaderValues {
        self.headers.index(name)
    }
}

#[derive(Debug)]
pub struct Sender {
    sender: cynthia::platform::channel::Sender<Trailers>,
}

impl Sender {
    #[doc(hidden)]
    pub fn new(sender: cynthia::platform::channel::Sender<Trailers>) -> Self {
        Self { sender }
    }

    pub async fn send(self, trailers: Trailers) {
        let _ = self.sender.send(trailers).await;
    }
}

#[must_use = "Futures do nothing unless polled or .awaited"]
#[derive(Debug)]
pub struct Receiver {
    receiver: cynthia::platform::channel::Receiver<Trailers>,
}

impl Receiver {
    pub(crate) fn new(receiver: cynthia::platform::channel::Receiver<Trailers>) -> Self {
        Self { receiver }
    }
}

impl Future for Receiver {
    type Output = Option<Trailers>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.receiver).poll_next(cx)
    }
}
