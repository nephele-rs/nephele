use std::{error, fmt, io};

use crate::proto::h2::frame::{Reason, StreamId};

#[derive(Debug)]
pub enum RecvError {
    Connection(Reason),
    Stream { id: StreamId, reason: Reason },
    Io(io::Error),
}

#[derive(Debug)]
pub enum SendError {
    User(UserError),
    Connection(Reason),
    Io(io::Error),
}

#[derive(Debug)]
pub enum UserError {
    InactiveStreamId,
    UnexpectedFrameType,
    PayloadTooBig,
    HeaderTooBig,
    Rejected,
    ReleaseCapacityTooBig,
    OverflowedStreamId,
    MalformedHeaders,
    MissingUriSchemeAndAuthority,
    PollResetAfterSendResponse,
    SendPingWhilePending,
    SendSettingsWhilePending,
    PeerDisabledServerPush,
}

impl From<io::Error> for RecvError {
    fn from(src: io::Error) -> Self {
        RecvError::Io(src)
    }
}

impl error::Error for RecvError {}

impl fmt::Display for RecvError {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        use self::RecvError::*;

        match *self {
            Connection(ref reason) => reason.fmt(fmt),
            Stream { ref reason, .. } => reason.fmt(fmt),
            Io(ref e) => e.fmt(fmt),
        }
    }
}

impl error::Error for SendError {}

impl fmt::Display for SendError {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        use self::SendError::*;

        match *self {
            User(ref e) => e.fmt(fmt),
            Connection(ref reason) => reason.fmt(fmt),
            Io(ref e) => e.fmt(fmt),
        }
    }
}

impl From<io::Error> for SendError {
    fn from(src: io::Error) -> Self {
        SendError::Io(src)
    }
}

impl From<UserError> for SendError {
    fn from(src: UserError) -> Self {
        SendError::User(src)
    }
}

impl error::Error for UserError {}

impl fmt::Display for UserError {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        use self::UserError::*;

        fmt.write_str(match *self {
            InactiveStreamId => "inactive stream",
            UnexpectedFrameType => "unexpected frame type",
            PayloadTooBig => "payload too big",
            HeaderTooBig => "header too big",
            Rejected => "rejected",
            ReleaseCapacityTooBig => "release capacity too big",
            OverflowedStreamId => "stream ID overflowed",
            MalformedHeaders => "malformed headers",
            MissingUriSchemeAndAuthority => "request URI missing scheme and authority",
            PollResetAfterSendResponse => "poll_reset after send_response is illegal",
            SendPingWhilePending => "send_ping before received previous pong",
            SendSettingsWhilePending => "sending SETTINGS before received previous ACK",
            PeerDisabledServerPush => "sending PUSH_PROMISE to peer who disabled server push",
        })
    }
}
