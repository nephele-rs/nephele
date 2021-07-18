use bytes::Bytes;
use std::fmt;

use super::Error;

pub fn strip_padding(payload: &mut Bytes) -> Result<u8, Error> {
    let payload_len = payload.len();
    if payload_len == 0 {
        return Err(Error::TooMuchPadding);
    }

    let pad_len = payload[0] as usize;

    if pad_len >= payload_len {
        return Err(Error::TooMuchPadding);
    }

    let _ = payload.split_to(1);
    let _ = payload.split_off(payload_len - pad_len - 1);

    Ok(pad_len as u8)
}

pub(super) fn debug_flags<'a, 'f: 'a>(
    fmt: &'a mut fmt::Formatter<'f>,
    bits: u8,
) -> DebugFlags<'a, 'f> {
    let result = write!(fmt, "({:#x}", bits);
    DebugFlags {
        fmt,
        result,
        started: false,
    }
}

pub(super) struct DebugFlags<'a, 'f: 'a> {
    fmt: &'a mut fmt::Formatter<'f>,
    result: fmt::Result,
    started: bool,
}

impl<'a, 'f: 'a> DebugFlags<'a, 'f> {
    pub(super) fn flag_if(&mut self, enabled: bool, name: &str) -> &mut Self {
        if enabled {
            self.result = self.result.and_then(|()| {
                let prefix = if self.started {
                    " | "
                } else {
                    self.started = true;
                    ": "
                };

                write!(self.fmt, "{}{}", prefix, name)
            });
        }
        self
    }

    pub(super) fn finish(&mut self) -> fmt::Result {
        self.result.and_then(|()| write!(self.fmt, ")"))
    }
}
