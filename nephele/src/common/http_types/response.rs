use cynthia::future::{prelude::*, swap};
use std::convert::{Into, TryInto};
use std::fmt::Debug;
use std::mem;
use std::ops::Index;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::common::http_types::convert::DeserializeOwned;
use crate::common::http_types::headers::{
    self, HeaderName, HeaderValue, HeaderValues, Headers, Names, ToHeaderValues, Values,
    CONTENT_TYPE,
};
use crate::common::http_types::mime::Mime;
use crate::common::http_types::trailers::{self, Trailers};
use crate::common::http_types::upgrade;
use crate::common::http_types::{Body, Extensions, StatusCode, Version};

pin_project_lite::pin_project! {
    #[derive(Debug)]
    pub struct Response {
        status: StatusCode,
        headers: Headers,
        version: Option<Version>,
        has_trailers: bool,
        trailers_sender: Option<cynthia::platform::channel::Sender<Trailers>>,
        trailers_receiver: Option<cynthia::platform::channel::Receiver<Trailers>>,
        upgrade_sender: Option<cynthia::platform::channel::Sender<upgrade::Connection>>,
        upgrade_receiver: Option<cynthia::platform::channel::Receiver<upgrade::Connection>>,
        has_upgrade: bool,
        #[pin]
        body: Body,
        ext: Extensions,
        local_addr: Option<String>,
        peer_addr: Option<String>,
    }
}

impl Response {
    pub fn new<S>(status: S) -> Self
    where
        S: TryInto<StatusCode>,
        S::Error: Debug,
    {
        let status = status
            .try_into()
            .expect("Could not convert into a valid `StatusCode`");
        let (trailers_sender, trailers_receiver) = cynthia::platform::channel::bounded(1);
        let (upgrade_sender, upgrade_receiver) = cynthia::platform::channel::bounded(1);
        Self {
            status,
            headers: Headers::new(),
            version: None,
            body: Body::empty(),
            trailers_sender: Some(trailers_sender),
            trailers_receiver: Some(trailers_receiver),
            has_trailers: false,
            upgrade_sender: Some(upgrade_sender),
            upgrade_receiver: Some(upgrade_receiver),
            has_upgrade: false,
            ext: Extensions::new(),
            peer_addr: None,
            local_addr: None,
        }
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn header_mut(&mut self, name: impl Into<HeaderName>) -> Option<&mut HeaderValues> {
        self.headers.get_mut(name.into())
    }

    pub fn header(&self, name: impl Into<HeaderName>) -> Option<&HeaderValues> {
        self.headers.get(name.into())
    }

    pub fn remove_header(&mut self, name: impl Into<HeaderName>) -> Option<HeaderValues> {
        self.headers.remove(name.into())
    }

    pub fn insert_header(
        &mut self,
        name: impl Into<HeaderName>,
        values: impl ToHeaderValues,
    ) -> Option<HeaderValues> {
        self.headers.insert(name, values)
    }

    pub fn append_header(&mut self, name: impl Into<HeaderName>, values: impl ToHeaderValues) {
        self.headers.append(name, values)
    }

    pub fn set_body(&mut self, body: impl Into<Body>) {
        self.replace_body(body);
    }

    pub fn replace_body(&mut self, body: impl Into<Body>) -> Body {
        let body = mem::replace(&mut self.body, body.into());
        self.copy_content_type_from_body();
        body
    }

    pub fn swap_body(&mut self, body: &mut Body) {
        mem::swap(&mut self.body, body);
        self.copy_content_type_from_body();
    }

    pub fn take_body(&mut self) -> Body {
        self.replace_body(Body::empty())
    }

    pub async fn body_string(&mut self) -> crate::common::http_types::Result<String> {
        let body = self.take_body();
        body.into_string().await
    }

    pub async fn body_bytes(&mut self) -> crate::common::http_types::Result<Vec<u8>> {
        let body = self.take_body();
        body.into_bytes().await
    }

    pub async fn body_json<T: DeserializeOwned>(&mut self) -> crate::common::http_types::Result<T> {
        let body = self.take_body();
        body.into_json().await
    }

    pub async fn body_form<T: DeserializeOwned>(&mut self) -> crate::common::http_types::Result<T> {
        let body = self.take_body();
        body.into_form().await
    }

    pub fn set_content_type(&mut self, mime: Mime) -> Option<HeaderValues> {
        let value: HeaderValue = mime.into();

        self.insert_header(CONTENT_TYPE, value)
    }

    fn copy_content_type_from_body(&mut self) {
        if self.header(CONTENT_TYPE).is_none() {
            self.set_content_type(self.body.mime().clone());
        }
    }

    pub fn content_type(&self) -> Option<Mime> {
        self.header(CONTENT_TYPE)?.last().as_str().parse().ok()
    }

    pub fn len(&self) -> Option<usize> {
        self.body.len()
    }

    pub fn is_empty(&self) -> Option<bool> {
        self.body.is_empty()
    }

    pub fn version(&self) -> Option<Version> {
        self.version
    }

    pub fn set_peer_addr(&mut self, peer_addr: Option<impl std::string::ToString>) {
        self.peer_addr = peer_addr.map(|addr| addr.to_string());
    }

    pub fn set_local_addr(&mut self, local_addr: Option<impl std::string::ToString>) {
        self.local_addr = local_addr.map(|addr| addr.to_string());
    }

    pub fn peer_addr(&self) -> Option<&str> {
        self.peer_addr.as_deref()
    }

    pub fn local_addr(&self) -> Option<&str> {
        self.local_addr.as_deref()
    }

    pub fn set_version(&mut self, version: Option<Version>) {
        self.version = version;
    }

    pub fn set_status(&mut self, status: StatusCode) {
        self.status = status;
    }

    pub fn send_trailers(&mut self) -> trailers::Sender {
        self.has_trailers = true;
        let sender = self
            .trailers_sender
            .take()
            .expect("Trailers sender can only be constructed once");
        trailers::Sender::new(sender)
    }

    pub fn recv_trailers(&mut self) -> trailers::Receiver {
        let receiver = self
            .trailers_receiver
            .take()
            .expect("Trailers receiver can only be constructed once");
        trailers::Receiver::new(receiver)
    }

    pub fn has_trailers(&self) -> bool {
        self.has_trailers
    }

    #[cfg_attr(feature = "docs", doc(cfg(unstable)))]
    pub fn send_upgrade(&mut self) -> upgrade::Sender {
        self.has_upgrade = true;
        let sender = self
            .upgrade_sender
            .take()
            .expect("Upgrade sender can only be constructed once");
        upgrade::Sender::new(sender)
    }

    #[cfg_attr(feature = "docs", doc(cfg(unstable)))]
    pub async fn recv_upgrade(&mut self) -> upgrade::Receiver {
        self.has_upgrade = true;
        let receiver = self
            .upgrade_receiver
            .take()
            .expect("Upgrade receiver can only be constructed once");
        upgrade::Receiver::new(receiver)
    }

    #[cfg_attr(feature = "docs", doc(cfg(unstable)))]
    pub fn has_upgrade(&self) -> bool {
        self.has_upgrade
    }

    pub fn iter(&self) -> headers::Iter<'_> {
        self.headers.iter()
    }

    pub fn iter_mut(&mut self) -> headers::IterMut<'_> {
        self.headers.iter_mut()
    }

    pub fn header_names(&self) -> Names<'_> {
        self.headers.names()
    }

    pub fn header_values(&self) -> Values<'_> {
        self.headers.values()
    }

    pub fn ext(&self) -> &Extensions {
        &self.ext
    }

    pub fn ext_mut(&mut self) -> &mut Extensions {
        &mut self.ext
    }
}

impl Clone for Response {
    fn clone(&self) -> Self {
        Self {
            status: self.status,
            headers: self.headers.clone(),
            version: self.version,
            trailers_sender: self.trailers_sender.clone(),
            trailers_receiver: self.trailers_receiver.clone(),
            has_trailers: false,
            upgrade_sender: self.upgrade_sender.clone(),
            upgrade_receiver: self.upgrade_receiver.clone(),
            has_upgrade: false,
            body: Body::empty(),
            ext: Extensions::new(),
            peer_addr: self.peer_addr.clone(),
            local_addr: self.local_addr.clone(),
        }
    }
}

impl AsyncRead for Response {
    #[allow(missing_doc_code_examples)]
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<swap::Result<usize>> {
        Pin::new(&mut self.body).poll_read(cx, buf)
    }
}

impl AsyncBufRead for Response {
    #[allow(missing_doc_code_examples)]
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<swap::Result<&'_ [u8]>> {
        let this = self.project();
        this.body.poll_fill_buf(cx)
    }

    fn consume(mut self: Pin<&mut Self>, amt: usize) {
        Pin::new(&mut self.body).consume(amt)
    }
}

impl AsRef<Headers> for Response {
    fn as_ref(&self) -> &Headers {
        &self.headers
    }
}

impl AsMut<Headers> for Response {
    fn as_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }
}

impl From<()> for Response {
    fn from(_: ()) -> Self {
        Response::new(StatusCode::NoContent)
    }
}
impl Index<HeaderName> for Response {
    type Output = HeaderValues;

    #[inline]
    fn index(&self, name: HeaderName) -> &HeaderValues {
        self.headers.index(name)
    }
}

impl Index<&str> for Response {
    type Output = HeaderValues;

    #[inline]
    fn index(&self, name: &str) -> &HeaderValues {
        self.headers.index(name)
    }
}

impl From<StatusCode> for Response {
    fn from(s: StatusCode) -> Self {
        Response::new(s)
    }
}

impl<T> From<T> for Response
where
    T: Into<Body>,
{
    fn from(value: T) -> Self {
        let body: Body = value.into();
        let mut res = Response::new(StatusCode::Ok);
        res.set_body(body);
        res
    }
}

impl IntoIterator for Response {
    type Item = (HeaderName, HeaderValues);
    type IntoIter = headers::IntoIter;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.headers.into_iter()
    }
}

impl<'a> IntoIterator for &'a Response {
    type Item = (&'a HeaderName, &'a HeaderValues);
    type IntoIter = headers::Iter<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.headers.iter()
    }
}

impl<'a> IntoIterator for &'a mut Response {
    type Item = (&'a HeaderName, &'a mut HeaderValues);
    type IntoIter = headers::IterMut<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.headers.iter_mut()
    }
}
