use super::HeaderName;

pub const CONNECTION: HeaderName = HeaderName::from_lowercase_str("connection");
pub const UPGRADE: HeaderName = HeaderName::from_lowercase_str("upgrade");
pub const CONTENT_TYPE: HeaderName = HeaderName::from_lowercase_str("content-type");
pub const HOST: HeaderName = HeaderName::from_lowercase_str("host");
pub const CONTENT_LENGTH: HeaderName = HeaderName::from_lowercase_str("content-length");
pub const DATE: HeaderName = HeaderName::from_lowercase_str("date");
pub const TRANSFER_ENCODING: HeaderName = HeaderName::from_lowercase_str("transfer-encoding");
pub const EXPECT: HeaderName = HeaderName::from_lowercase_str("expect");
