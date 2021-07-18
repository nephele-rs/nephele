use core::convert::{Infallible, TryInto};
use std::error::Error as StdError;
use std::fmt::Debug;

use crate::common::http_types::{Error, StatusCode};

pub trait Status<T, E>: private::Sealed {
    fn status<S>(self, status: S) -> Result<T, Error>
    where
        S: TryInto<StatusCode>,
        S::Error: Debug;

    fn with_status<S, F>(self, f: F) -> Result<T, Error>
    where
        S: TryInto<StatusCode>,
        S::Error: Debug,
        F: FnOnce() -> S;
}

impl<T, E> Status<T, E> for Result<T, E>
where
    E: StdError + Send + Sync + 'static,
{
    fn status<S>(self, status: S) -> Result<T, Error>
    where
        S: TryInto<StatusCode>,
        S::Error: Debug,
    {
        self.map_err(|error| {
            let status = status
                .try_into()
                .expect("Could not convert into a valid `StatusCode`");
            Error::new(status, error)
        })
    }

    fn with_status<S, F>(self, f: F) -> Result<T, Error>
    where
        S: TryInto<StatusCode>,
        S::Error: Debug,
        F: FnOnce() -> S,
    {
        self.map_err(|error| {
            let status = f()
                .try_into()
                .expect("Could not convert into a valid `StatusCode`");
            Error::new(status, error)
        })
    }
}

impl<T> Status<T, Infallible> for Option<T> {
    fn status<S>(self, status: S) -> Result<T, Error>
    where
        S: TryInto<StatusCode>,
        S::Error: Debug,
    {
        self.ok_or_else(|| {
            let status = status
                .try_into()
                .expect("Could not convert into a valid `StatusCode`");
            Error::from_str(status, "NoneError")
        })
    }

    fn with_status<S, F>(self, f: F) -> Result<T, Error>
    where
        S: TryInto<StatusCode>,
        S::Error: Debug,
        F: FnOnce() -> S,
    {
        self.ok_or_else(|| {
            let status = f()
                .try_into()
                .expect("Could not convert into a valid `StatusCode`");
            Error::from_str(status, "NoneError")
        })
    }
}

pub(crate) mod private {
    pub trait Sealed {}

    impl<T, E> Sealed for Result<T, E> {}
    impl<T> Sealed for Option<T> {}
}
