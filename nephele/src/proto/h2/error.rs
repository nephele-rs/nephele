use std::{error, fmt, io};

use crate::proto::h2::codec::{SendError, UserError};
pub use crate::proto::h2::frame::Reason;
use crate::proto::h2::proto;

#[derive(Debug)]
pub struct Error {
    kind: Kind,
}

#[derive(Debug)]
enum Kind {
    Proto(Reason),
    User(UserError),
    Io(io::Error),
}

impl Error {
    pub fn reason(&self) -> Option<Reason> {
        match self.kind {
            Kind::Proto(reason) => Some(reason),
            _ => None,
        }
    }

    pub fn is_io(&self) -> bool {
        match self.kind {
            Kind::Io(_) => true,
            _ => false,
        }
    }

    pub fn get_io(&self) -> Option<&io::Error> {
        match self.kind {
            Kind::Io(ref e) => Some(e),
            _ => None,
        }
    }

    pub fn into_io(self) -> Option<io::Error> {
        match self.kind {
            Kind::Io(e) => Some(e),
            _ => None,
        }
    }

    pub(crate) fn from_io(err: io::Error) -> Self {
        Error {
            kind: Kind::Io(err),
        }
    }
}

impl From<proto::Error> for Error {
    fn from(src: proto::Error) -> Error {
        use crate::proto::h2::proto::Error::*;

        Error {
            kind: match src {
                Proto(reason) => Kind::Proto(reason),
                Io(e) => Kind::Io(e),
            },
        }
    }
}

impl From<Reason> for Error {
    fn from(src: Reason) -> Error {
        Error {
            kind: Kind::Proto(src),
        }
    }
}

impl From<SendError> for Error {
    fn from(src: SendError) -> Error {
        match src {
            SendError::User(e) => e.into(),
            SendError::Connection(reason) => reason.into(),
            SendError::Io(e) => Error::from_io(e),
        }
    }
}

impl From<UserError> for Error {
    fn from(src: UserError) -> Error {
        Error {
            kind: Kind::User(src),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        use self::Kind::*;

        match self.kind {
            Proto(ref reason) => write!(fmt, "protocol error: {}", reason),
            User(ref e) => write!(fmt, "user error: {}", e),
            Io(ref e) => fmt::Display::fmt(e, fmt),
        }
    }
}

impl error::Error for Error {}
