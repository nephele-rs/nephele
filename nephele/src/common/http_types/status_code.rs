use serde::de::{Error as DeError, Unexpected, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::{self, Display};

#[repr(u16)]
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum StatusCode {
    Continue = 100,

    SwitchingProtocols = 101,

    EarlyHints = 103,

    Ok = 200,

    Created = 201,

    Accepted = 202,

    NonAuthoritativeInformation = 203,

    NoContent = 204,

    ResetContent = 205,

    PartialContent = 206,

    MultiStatus = 207,

    ImUsed = 226,

    MultipleChoice = 300,

    MovedPermanently = 301,

    Found = 302,

    SeeOther = 303,

    NotModified = 304,

    TemporaryRedirect = 307,

    PermanentRedirect = 308,

    BadRequest = 400,

    Unauthorized = 401,

    PaymentRequired = 402,

    Forbidden = 403,

    NotFound = 404,

    MethodNotAllowed = 405,

    NotAcceptable = 406,

    ProxyAuthenticationRequired = 407,

    RequestTimeout = 408,

    Conflict = 409,

    Gone = 410,

    LengthRequired = 411,

    PreconditionFailed = 412,

    PayloadTooLarge = 413,

    UriTooLong = 414,

    UnsupportedMediaType = 415,

    RequestedRangeNotSatisfiable = 416,

    ExpectationFailed = 417,

    ImATeapot = 418,

    MisdirectedRequest = 421,

    UnprocessableEntity = 422,

    Locked = 423,

    FailedDependency = 424,

    TooEarly = 425,

    UpgradeRequired = 426,

    PreconditionRequired = 428,

    TooManyRequests = 429,

    RequestHeaderFieldsTooLarge = 431,

    UnavailableForLegalReasons = 451,

    InternalServerError = 500,

    NotImplemented = 501,

    BadGateway = 502,

    ServiceUnavailable = 503,

    GatewayTimeout = 504,

    HttpVersionNotSupported = 505,

    VariantAlsoNegotiates = 506,

    InsufficientStorage = 507,

    LoopDetected = 508,

    NotExtended = 510,

    NetworkAuthenticationRequired = 511,
}

impl StatusCode {
    pub fn is_informational(&self) -> bool {
        let num: u16 = self.clone().into();
        num >= 100 && num < 200
    }

    pub fn is_success(&self) -> bool {
        let num: u16 = self.clone().into();
        num >= 200 && num < 300
    }

    pub fn is_redirection(&self) -> bool {
        let num: u16 = self.clone().into();
        num >= 300 && num < 400
    }

    pub fn is_client_error(&self) -> bool {
        let num: u16 = self.clone().into();
        num >= 400 && num < 500
    }

    pub fn is_server_error(&self) -> bool {
        let num: u16 = self.clone().into();
        num >= 500 && num < 600
    }

    pub fn canonical_reason(&self) -> &'static str {
        match self {
            StatusCode::Continue => "Continue",
            StatusCode::SwitchingProtocols => "Switching Protocols",
            StatusCode::EarlyHints => "Early Hints",
            StatusCode::Ok => "OK",
            StatusCode::Created => "Created",
            StatusCode::Accepted => "Accepted",
            StatusCode::NonAuthoritativeInformation => "Non Authoritative Information",
            StatusCode::NoContent => "No Content",
            StatusCode::ResetContent => "Reset Content",
            StatusCode::PartialContent => "Partial Content",
            StatusCode::MultiStatus => "Multi-Status",
            StatusCode::ImUsed => "Im Used",
            StatusCode::MultipleChoice => "Multiple Choice",
            StatusCode::MovedPermanently => "Moved Permanently",
            StatusCode::Found => "Found",
            StatusCode::SeeOther => "See Other",
            StatusCode::NotModified => "Modified",
            StatusCode::TemporaryRedirect => "Temporary Redirect",
            StatusCode::PermanentRedirect => "Permanent Redirect",
            StatusCode::BadRequest => "Bad Request",
            StatusCode::Unauthorized => "Unauthorized",
            StatusCode::PaymentRequired => "Payment Required",
            StatusCode::Forbidden => "Forbidden",
            StatusCode::NotFound => "Not Found",
            StatusCode::MethodNotAllowed => "Method Not Allowed",
            StatusCode::NotAcceptable => "Not Acceptable",
            StatusCode::ProxyAuthenticationRequired => "Proxy Authentication Required",
            StatusCode::RequestTimeout => "Request Timeout",
            StatusCode::Conflict => "Conflict",
            StatusCode::Gone => "Gone",
            StatusCode::LengthRequired => "Length Required",
            StatusCode::PreconditionFailed => "Precondition Failed",
            StatusCode::PayloadTooLarge => "Payload Too Large",
            StatusCode::UriTooLong => "URI Too Long",
            StatusCode::UnsupportedMediaType => "Unsupported Media Type",
            StatusCode::RequestedRangeNotSatisfiable => "Requested Range Not Satisfiable",
            StatusCode::ExpectationFailed => "Expectation Failed",
            StatusCode::ImATeapot => "I'm a teapot",
            StatusCode::MisdirectedRequest => "Misdirected Request",
            StatusCode::UnprocessableEntity => "Unprocessable Entity",
            StatusCode::Locked => "Locked",
            StatusCode::FailedDependency => "Failed Dependency",
            StatusCode::TooEarly => "Too Early",
            StatusCode::UpgradeRequired => "Upgrade Required",
            StatusCode::PreconditionRequired => "Precondition Required",
            StatusCode::TooManyRequests => "Too Many Requests",
            StatusCode::RequestHeaderFieldsTooLarge => "Request Header Fields Too Large",
            StatusCode::UnavailableForLegalReasons => "Unavailable For Legal Reasons",
            StatusCode::InternalServerError => "Internal Server Error",
            StatusCode::NotImplemented => "Not Implemented",
            StatusCode::BadGateway => "Bad Gateway",
            StatusCode::ServiceUnavailable => "Service Unavailable",
            StatusCode::GatewayTimeout => "Gateway Timeout",
            StatusCode::HttpVersionNotSupported => "HTTP Version Not Supported",
            StatusCode::VariantAlsoNegotiates => "Variant Also Negotiates",
            StatusCode::InsufficientStorage => "Insufficient Storage",
            StatusCode::LoopDetected => "Loop Detected",
            StatusCode::NotExtended => "Not Extended",
            StatusCode::NetworkAuthenticationRequired => "Network Authentication Required",
        }
    }
}

impl Serialize for StatusCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let value: u16 = *self as u16;
        serializer.serialize_u16(value)
    }
}

struct StatusCodeU16Visitor;

impl<'de> Visitor<'de> for StatusCodeU16Visitor {
    type Value = StatusCode;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "a u16 representing the status code")
    }

    fn visit_i16<E>(self, v: i16) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        self.visit_u16(v as u16)
    }

    fn visit_i32<E>(self, v: i32) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        self.visit_u16(v as u16)
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        self.visit_u16(v as u16)
    }

    fn visit_u16<E>(self, v: u16) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        use std::convert::TryFrom;
        match StatusCode::try_from(v) {
            Ok(status_code) => Ok(status_code),
            Err(_) => Err(DeError::invalid_value(
                Unexpected::Unsigned(v as u64),
                &self,
            )),
        }
    }

    fn visit_u32<E>(self, v: u32) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        self.visit_u16(v as u16)
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        self.visit_u16(v as u16)
    }
}

impl<'de> Deserialize<'de> for StatusCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StatusCodeU16Visitor)
    }
}

impl From<StatusCode> for u16 {
    fn from(code: StatusCode) -> u16 {
        code as u16
    }
}

impl std::convert::TryFrom<u16> for StatusCode {
    type Error = super::Error;

    fn try_from(num: u16) -> Result<Self, Self::Error> {
        match num {
            100 => Ok(StatusCode::Continue),
            101 => Ok(StatusCode::SwitchingProtocols),
            103 => Ok(StatusCode::EarlyHints),
            200 => Ok(StatusCode::Ok),
            201 => Ok(StatusCode::Created),
            202 => Ok(StatusCode::Accepted),
            203 => Ok(StatusCode::NonAuthoritativeInformation),
            204 => Ok(StatusCode::NoContent),
            205 => Ok(StatusCode::ResetContent),
            206 => Ok(StatusCode::PartialContent),
            207 => Ok(StatusCode::MultiStatus),
            226 => Ok(StatusCode::ImUsed),
            300 => Ok(StatusCode::MultipleChoice),
            301 => Ok(StatusCode::MovedPermanently),
            302 => Ok(StatusCode::Found),
            303 => Ok(StatusCode::SeeOther),
            304 => Ok(StatusCode::NotModified),
            307 => Ok(StatusCode::TemporaryRedirect),
            308 => Ok(StatusCode::PermanentRedirect),
            400 => Ok(StatusCode::BadRequest),
            401 => Ok(StatusCode::Unauthorized),
            402 => Ok(StatusCode::PaymentRequired),
            403 => Ok(StatusCode::Forbidden),
            404 => Ok(StatusCode::NotFound),
            405 => Ok(StatusCode::MethodNotAllowed),
            406 => Ok(StatusCode::NotAcceptable),
            407 => Ok(StatusCode::ProxyAuthenticationRequired),
            408 => Ok(StatusCode::RequestTimeout),
            409 => Ok(StatusCode::Conflict),
            410 => Ok(StatusCode::Gone),
            411 => Ok(StatusCode::LengthRequired),
            412 => Ok(StatusCode::PreconditionFailed),
            413 => Ok(StatusCode::PayloadTooLarge),
            414 => Ok(StatusCode::UriTooLong),
            415 => Ok(StatusCode::UnsupportedMediaType),
            416 => Ok(StatusCode::RequestedRangeNotSatisfiable),
            417 => Ok(StatusCode::ExpectationFailed),
            418 => Ok(StatusCode::ImATeapot),
            421 => Ok(StatusCode::MisdirectedRequest),
            422 => Ok(StatusCode::UnprocessableEntity),
            423 => Ok(StatusCode::Locked),
            424 => Ok(StatusCode::FailedDependency),
            425 => Ok(StatusCode::TooEarly),
            426 => Ok(StatusCode::UpgradeRequired),
            428 => Ok(StatusCode::PreconditionRequired),
            429 => Ok(StatusCode::TooManyRequests),
            431 => Ok(StatusCode::RequestHeaderFieldsTooLarge),
            451 => Ok(StatusCode::UnavailableForLegalReasons),
            500 => Ok(StatusCode::InternalServerError),
            501 => Ok(StatusCode::NotImplemented),
            502 => Ok(StatusCode::BadGateway),
            503 => Ok(StatusCode::ServiceUnavailable),
            504 => Ok(StatusCode::GatewayTimeout),
            505 => Ok(StatusCode::HttpVersionNotSupported),
            506 => Ok(StatusCode::VariantAlsoNegotiates),
            507 => Ok(StatusCode::InsufficientStorage),
            508 => Ok(StatusCode::LoopDetected),
            510 => Ok(StatusCode::NotExtended),
            511 => Ok(StatusCode::NetworkAuthenticationRequired),
            _ => crate::bail!("Invalid status code"),
        }
    }
}

impl PartialEq<StatusCode> for u16 {
    fn eq(&self, other: &StatusCode) -> bool {
        *self == *other as u16
    }
}

impl PartialEq<u16> for StatusCode {
    fn eq(&self, other: &u16) -> bool {
        *self as u16 == *other
    }
}

impl Display for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", *self as u16)
    }
}
