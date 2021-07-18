mod constants;
mod parse;

use std::borrow::Cow;
use std::fmt::{self, Debug, Display};
use std::option;
use std::str::FromStr;

use crate::common::http_types::headers::{HeaderValue, ToHeaderValues};
pub use crate::common::http_types::mime::constants::*;

use infer::Infer;

#[derive(Clone)]
pub struct Mime {
    pub(crate) essence: String,
    pub(crate) basetype: String,
    pub(crate) subtype: String,
    pub(crate) static_essence: Option<&'static str>,
    pub(crate) static_basetype: Option<&'static str>,
    pub(crate) static_subtype: Option<&'static str>,
    pub(crate) params: Option<ParamKind>,
}

impl Mime {
    pub fn sniff(bytes: &[u8]) -> crate::common::http_types::Result<Self> {
        let info = Infer::new();
        let mime = match info.get(&bytes) {
            Some(info) => info.mime,
            None => crate::bail!("Could not sniff the mime type"),
        };
        Mime::from_str(&mime)
    }

    pub fn from_extension(extension: impl AsRef<str>) -> Option<Self> {
        match extension.as_ref() {
            "html" => Some(HTML),
            "js" | "mjs" | "jsonp" => Some(JAVASCRIPT),
            "json" => Some(JSON),
            "css" => Some(CSS),
            "svg" => Some(SVG),
            "xml" => Some(XML),
            _ => None,
        }
    }

    pub fn basetype(&self) -> &str {
        if let Some(basetype) = self.static_basetype {
            &basetype
        } else {
            &self.basetype
        }
    }

    pub fn subtype(&self) -> &str {
        if let Some(subtype) = self.static_subtype {
            &subtype
        } else {
            &self.subtype
        }
    }

    pub fn essence(&self) -> &str {
        if let Some(essence) = self.static_essence {
            &essence
        } else {
            &self.essence
        }
    }

    pub fn param(&self, name: impl Into<ParamName>) -> Option<&ParamValue> {
        let name: ParamName = name.into();
        self.params
            .as_ref()
            .map(|inner| match inner {
                ParamKind::Vec(v) => v
                    .iter()
                    .find_map(|(k, v)| if k == &name { Some(v) } else { None }),
                ParamKind::Utf8 => match name {
                    ParamName(Cow::Borrowed("charset")) => Some(&ParamValue(Cow::Borrowed("utf8"))),
                    _ => None,
                },
            })
            .flatten()
    }

    pub fn remove_param(&mut self, name: impl Into<ParamName>) -> Option<ParamValue> {
        let name: ParamName = name.into();
        let mut unset_params = false;
        let ret = self
            .params
            .as_mut()
            .map(|inner| match inner {
                ParamKind::Vec(v) => match v.iter().position(|(k, _)| k == &name) {
                    Some(index) => Some(v.remove(index).1),
                    None => None,
                },
                ParamKind::Utf8 => match name {
                    ParamName(Cow::Borrowed("charset")) => {
                        unset_params = true;
                        Some(ParamValue(Cow::Borrowed("utf8")))
                    }
                    _ => None,
                },
            })
            .flatten();
        if unset_params {
            self.params = None;
        }
        ret
    }
}

impl PartialEq<Mime> for Mime {
    fn eq(&self, other: &Mime) -> bool {
        let left = match self.static_essence {
            Some(essence) => essence,
            None => &self.essence,
        };
        let right = match other.static_essence {
            Some(essence) => essence,
            None => &other.essence,
        };
        left == right
    }
}

impl Display for Mime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        parse::format(self, f)
    }
}

impl Debug for Mime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(essence) = self.static_essence {
            Debug::fmt(essence, f)
        } else {
            Debug::fmt(&self.essence, f)
        }
    }
}

impl FromStr for Mime {
    type Err = crate::common::http_types::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse::parse(s)
    }
}

impl<'a> From<&'a str> for Mime {
    fn from(value: &'a str) -> Self {
        Self::from_str(value).unwrap()
    }
}

impl ToHeaderValues for Mime {
    type Iter = option::IntoIter<HeaderValue>;

    fn to_header_values(&self) -> crate::common::http_types::Result<Self::Iter> {
        let mime = self.clone();
        let header: HeaderValue = mime.into();

        Ok(header.to_header_values().unwrap())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParamName(Cow<'static, str>);

impl ParamName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for ParamName {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl FromStr for ParamName {
    type Err = crate::common::http_types::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        crate::ensure!(s.is_ascii(), "String slice should be valid ASCII");
        Ok(ParamName(Cow::Owned(s.to_ascii_lowercase())))
    }
}

impl<'a> From<&'a str> for ParamName {
    fn from(value: &'a str) -> Self {
        Self::from_str(value).unwrap()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParamValue(Cow<'static, str>);

impl ParamValue {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for ParamValue {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl<'a> PartialEq<&'a str> for ParamValue {
    fn eq(&self, other: &&'a str) -> bool {
        &self.0 == other
    }
}

impl PartialEq<str> for ParamValue {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParamKind {
    Utf8,
    Vec(Vec<(ParamName, ParamValue)>),
}
