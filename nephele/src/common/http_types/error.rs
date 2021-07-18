use std::convert::TryInto;
use std::error::Error as StdError;
use std::fmt::{self, Debug, Display};

use crate::common::http_types::StatusCode;

pub type Result<T> = std::result::Result<T, Error>;

pub struct Error {
    error: anyhow::Error,
    status: crate::common::http_types::StatusCode,
    type_name: Option<&'static str>,
}

#[allow(unreachable_pub)]
#[derive(Debug)]
#[doc(hidden)]
pub struct BacktracePlaceholder;

impl Display for BacktracePlaceholder {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        unreachable!()
    }
}

impl Error {
    pub fn new<S, E>(status: S, error: E) -> Self
    where
        S: TryInto<StatusCode>,
        S::Error: Debug,
        E: Into<anyhow::Error>,
    {
        Self {
            status: status
                .try_into()
                .expect("Could not convert into a valid `StatusCode`"),
            error: error.into(),
            type_name: Some(std::any::type_name::<E>()),
        }
    }

    pub fn from_str<S, M>(status: S, msg: M) -> Self
    where
        S: TryInto<StatusCode>,
        S::Error: Debug,
        M: Display + Debug + Send + Sync + 'static,
    {
        Self {
            status: status
                .try_into()
                .expect("Could not convert into a valid `StatusCode`"),
            error: anyhow::Error::msg(msg),
            type_name: None,
        }
    }

    pub(crate) fn new_adhoc<M>(message: M) -> Error
    where
        M: Display + Debug + Send + Sync + 'static,
    {
        Self::from_str(StatusCode::InternalServerError, message)
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn set_status<S>(&mut self, status: S)
    where
        S: TryInto<StatusCode>,
        S::Error: Debug,
    {
        self.status = status
            .try_into()
            .expect("Could not convert into a valid `StatusCode`");
    }

    #[cfg(backtrace)]
    pub fn backtrace(&self) -> Option<&std::backtrace::Backtrace> {
        let backtrace = self.error.backtrace();
        if let std::backtrace::BacktraceStatus::Captured = backtrace.status() {
            Some(backtrace)
        } else {
            None
        }
    }

    #[cfg(not(backtrace))]
    #[allow(missing_docs)]
    pub const fn backtrace(&self) -> Option<BacktracePlaceholder> {
        None
    }

    pub fn into_inner(self) -> anyhow::Error {
        self.error
    }

    pub fn downcast<E>(self) -> std::result::Result<E, Self>
    where
        E: Display + Debug + Send + Sync + 'static,
    {
        if self.error.downcast_ref::<E>().is_some() {
            Ok(self.error.downcast().unwrap())
        } else {
            Err(self)
        }
    }

    pub fn downcast_ref<E>(&self) -> Option<&E>
    where
        E: Display + Debug + Send + Sync + 'static,
    {
        self.error.downcast_ref::<E>()
    }

    pub fn downcast_mut<E>(&mut self) -> Option<&mut E>
    where
        E: Display + Debug + Send + Sync + 'static,
    {
        self.error.downcast_mut::<E>()
    }

    pub fn type_name(&self) -> Option<&str> {
        self.type_name.as_deref()
    }
}

impl Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.error, formatter)
    }
}

impl Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        Debug::fmt(&self.error, formatter)
    }
}

impl<E: Into<anyhow::Error>> From<E> for Error {
    fn from(error: E) -> Self {
        Self::new(StatusCode::InternalServerError, error)
    }
}
impl AsRef<dyn StdError + Send + Sync> for Error {
    fn as_ref(&self) -> &(dyn StdError + Send + Sync + 'static) {
        self.error.as_ref()
    }
}

impl AsRef<StatusCode> for Error {
    fn as_ref(&self) -> &StatusCode {
        &self.status
    }
}

impl AsMut<StatusCode> for Error {
    fn as_mut(&mut self) -> &mut StatusCode {
        &mut self.status
    }
}

impl AsRef<dyn StdError> for Error {
    fn as_ref(&self) -> &(dyn StdError + 'static) {
        self.error.as_ref()
    }
}

impl From<Error> for Box<dyn StdError + Send + Sync + 'static> {
    fn from(error: Error) -> Self {
        error.error.into()
    }
}

impl From<Error> for Box<dyn StdError + 'static> {
    fn from(error: Error) -> Self {
        Box::<dyn StdError + Send + Sync>::from(error.error)
    }
}
