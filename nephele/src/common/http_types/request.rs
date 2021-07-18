use cynthia::future::{prelude::*, swap};
use std::convert::{Into, TryInto};
use std::mem;
use std::ops::Index;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::common::http_types::convert::{DeserializeOwned, Serialize};
use crate::common::http_types::headers::{
    self, HeaderName, HeaderValue, HeaderValues, Headers, Names, ToHeaderValues, Values,
    CONTENT_TYPE,
};
use crate::common::http_types::mime::Mime;
use crate::common::http_types::trailers::{self, Trailers};
use crate::common::http_types::{Body, Extensions, Method, StatusCode, Url, Version};

pin_project_lite::pin_project! {
    #[derive(Debug)]
    pub struct Request {
        method: Method,
        url: Url,
        headers: Headers,
        version: Option<Version>,
        #[pin]
        body: Body,
        local_addr: Option<String>,
        peer_addr: Option<String>,
        ext: Extensions,
        trailers_sender: Option<cynthia::platform::channel::Sender<Trailers>>,
        trailers_receiver: Option<cynthia::platform::channel::Receiver<Trailers>>,
        has_trailers: bool,
    }
}

impl Request {
    pub fn new<U>(method: Method, url: U) -> Self
    where
        U: TryInto<Url>,
        U::Error: std::fmt::Debug,
    {
        let url = url.try_into().expect("Could not convert into a valid url");
        let (trailers_sender, trailers_receiver) = cynthia::platform::channel::bounded(1);
        Self {
            method,
            url,
            headers: Headers::new(),
            version: None,
            body: Body::empty(),
            ext: Extensions::new(),
            peer_addr: None,
            local_addr: None,
            trailers_receiver: Some(trailers_receiver),
            trailers_sender: Some(trailers_sender),
            has_trailers: false,
        }
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

    pub fn remote(&self) -> Option<&str> {
        self.forwarded_for().or_else(|| self.peer_addr())
    }

    pub fn host(&self) -> Option<&str> {
        self.forwarded_header_part("host")
            .or_else(|| {
                self.header("X-Forwarded-Host")
                    .and_then(|h| h.as_str().split(',').next())
            })
            .or_else(|| self.header(&headers::HOST).map(|h| h.as_str()))
            .or_else(|| self.url().host_str())
    }

    fn forwarded_header_part(&self, part: &str) -> Option<&str> {
        self.header("Forwarded").and_then(|header| {
            header.as_str().split(';').find_map(|key_equals_value| {
                let parts = key_equals_value.split('=').collect::<Vec<_>>();
                if parts.len() == 2 && parts[0].eq_ignore_ascii_case(part) {
                    Some(parts[1])
                } else {
                    None
                }
            })
        })
    }

    fn forwarded_for(&self) -> Option<&str> {
        self.forwarded_header_part("for").or_else(|| {
            self.header("X-Forwarded-For")
                .and_then(|header| header.as_str().split(',').next())
        })
    }

    pub fn method(&self) -> Method {
        self.method
    }

    pub fn set_method(&mut self, method: Method) {
        self.method = method;
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn url_mut(&mut self) -> &mut Url {
        &mut self.url
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

    pub fn header(&self, name: impl Into<HeaderName>) -> Option<&HeaderValues> {
        self.headers.get(name)
    }

    pub fn header_mut(&mut self, name: impl Into<HeaderName>) -> Option<&mut HeaderValues> {
        self.headers.get_mut(name.into())
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

    pub fn set_version(&mut self, version: Option<Version>) {
        self.version = version;
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

    pub fn query<T: serde::de::DeserializeOwned>(&self) -> crate::common::http_types::Result<T> {
        let query = self.url().query().unwrap_or("");
        serde_qs::from_str(query).map_err(|e| {
            crate::common::http_types::Error::from_str(StatusCode::BadRequest, format!("{}", e))
        })
    }

    pub fn set_query(&mut self, query: &impl Serialize) -> crate::common::http_types::Result<()> {
        let query = serde_qs::to_string(query).map_err(|e| {
            crate::common::http_types::Error::from_str(StatusCode::BadRequest, format!("{}", e))
        })?;
        self.url.set_query(Some(&query));
        Ok(())
    }

    pub fn get<U>(url: U) -> Self
    where
        U: TryInto<Url>,
        U::Error: std::fmt::Debug,
    {
        Request::new(Method::Get, url)
    }

    pub fn head<U>(url: U) -> Self
    where
        U: TryInto<Url>,
        U::Error: std::fmt::Debug,
    {
        Request::new(Method::Head, url)
    }

    pub fn post<U>(url: U) -> Self
    where
        U: TryInto<Url>,
        U::Error: std::fmt::Debug,
    {
        Request::new(Method::Post, url)
    }

    pub fn put<U>(url: U) -> Self
    where
        U: TryInto<Url>,
        U::Error: std::fmt::Debug,
    {
        Request::new(Method::Put, url)
    }

    pub fn delete<U>(url: U) -> Self
    where
        U: TryInto<Url>,
        U::Error: std::fmt::Debug,
    {
        Request::new(Method::Delete, url)
    }

    pub fn connect<U>(url: U) -> Self
    where
        U: TryInto<Url>,
        U::Error: std::fmt::Debug,
    {
        Request::new(Method::Connect, url)
    }

    pub fn options<U>(url: U) -> Self
    where
        U: TryInto<Url>,
        U::Error: std::fmt::Debug,
    {
        Request::new(Method::Options, url)
    }

    pub fn trace<U>(url: U) -> Self
    where
        U: TryInto<Url>,
        U::Error: std::fmt::Debug,
    {
        Request::new(Method::Trace, url)
    }

    pub fn patch<U>(url: U) -> Self
    where
        U: TryInto<Url>,
        U::Error: std::fmt::Debug,
    {
        Request::new(Method::Patch, url)
    }
}

impl Clone for Request {
    fn clone(&self) -> Self {
        Request {
            method: self.method,
            url: self.url.clone(),
            headers: self.headers.clone(),
            version: self.version,
            trailers_sender: None,
            trailers_receiver: None,
            body: Body::empty(),
            ext: Extensions::new(),
            peer_addr: self.peer_addr.clone(),
            local_addr: self.local_addr.clone(),
            has_trailers: false,
        }
    }
}

impl AsyncRead for Request {
    #[allow(missing_doc_code_examples)]
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<swap::Result<usize>> {
        Pin::new(&mut self.body).poll_read(cx, buf)
    }
}

impl AsyncBufRead for Request {
    #[allow(missing_doc_code_examples)]
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<swap::Result<&'_ [u8]>> {
        let this = self.project();
        this.body.poll_fill_buf(cx)
    }

    fn consume(mut self: Pin<&mut Self>, length: usize) {
        Pin::new(&mut self.body).consume(length)
    }
}

impl AsRef<Headers> for Request {
    fn as_ref(&self) -> &Headers {
        &self.headers
    }
}

impl AsMut<Headers> for Request {
    fn as_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }
}

impl From<Request> for Body {
    fn from(req: Request) -> Body {
        req.body
    }
}

impl Index<HeaderName> for Request {
    type Output = HeaderValues;

    #[inline]
    fn index(&self, name: HeaderName) -> &HeaderValues {
        self.headers.index(name)
    }
}

impl Index<&str> for Request {
    type Output = HeaderValues;

    #[inline]
    fn index(&self, name: &str) -> &HeaderValues {
        self.headers.index(name)
    }
}

impl IntoIterator for Request {
    type Item = (HeaderName, HeaderValues);
    type IntoIter = headers::IntoIter;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.headers.into_iter()
    }
}

impl<'a> IntoIterator for &'a Request {
    type Item = (&'a HeaderName, &'a HeaderValues);
    type IntoIter = headers::Iter<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.headers.iter()
    }
}

impl<'a> IntoIterator for &'a mut Request {
    type Item = (&'a HeaderName, &'a mut HeaderValues);
    type IntoIter = headers::IterMut<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.headers.iter_mut()
    }
}
