pub mod convert {
    pub use serde::{de::DeserializeOwned, Deserialize, Serialize};
    #[doc(inline)]
    pub use serde_json::json;
}

pub use trailers::Trailers;
pub mod trailers;

pub mod url {
    pub use url::{
        EncodingOverride, Host, OpaqueOrigin, Origin, ParseError, ParseOptions, PathSegmentsMut,
        Position, SyntaxViolation, Url, UrlQuery,
    };
}

#[doc(inline)]
pub use crate::common::http_types::url::Url;

mod error;

mod body;
mod extensions;
mod macros;
mod method;
mod parse_utils;
mod request;
mod response;
mod status;
mod status_code;
mod version;

pub mod content;
pub mod headers;
pub mod mime;
pub mod upgrade;

pub use body::Body;
pub use error::{Error, Result};
pub use method::Method;
pub use request::Request;
pub use response::Response;
pub use status::Status;
pub use status_code::StatusCode;
pub use version::Version;

#[doc(inline)]
pub use extensions::Extensions;

#[doc(inline)]
pub use mime::Mime;

#[doc(hidden)]
pub mod private {
    use crate::common::http_types::Error;
    pub use crate::common::http_types::StatusCode;
    use core::fmt::{Debug, Display};
    pub use core::result::Result::Err;

    pub fn new_adhoc<M>(message: M) -> Error
    where
        M: Display + Debug + Send + Sync + 'static,
    {
        Error::new_adhoc(message)
    }
}
