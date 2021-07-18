use http::{HeaderMap, Request, Response};
use std::fmt;

use crate::proto::h2::codec::RecvError;
use crate::proto::h2::error::Reason;
use crate::proto::h2::frame::{Pseudo, StreamId};
use crate::proto::h2::proto::Open;

pub(crate) trait Peer {
    type Poll: fmt::Debug;
    const NAME: &'static str;

    fn r#dyn() -> Dyn;

    fn is_server() -> bool;

    fn convert_poll_message(
        pseudo: Pseudo,
        fields: HeaderMap,
        stream_id: StreamId,
    ) -> Result<Self::Poll, RecvError>;

    fn is_local_init(id: StreamId) -> bool {
        assert!(!id.is_zero());
        Self::is_server() == id.is_server_initiated()
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) enum Dyn {
    Client,
    Server,
}

#[derive(Debug)]
pub enum PollMessage {
    Client(Response<()>),
    Server(Request<()>),
}

impl Dyn {
    pub fn is_server(&self) -> bool {
        *self == Dyn::Server
    }

    pub fn is_local_init(&self, id: StreamId) -> bool {
        assert!(!id.is_zero());
        self.is_server() == id.is_server_initiated()
    }

    pub fn convert_poll_message(
        &self,
        pseudo: Pseudo,
        fields: HeaderMap,
        stream_id: StreamId,
    ) -> Result<PollMessage, RecvError> {
        if self.is_server() {
            crate::proto::h2::server::Peer::convert_poll_message(pseudo, fields, stream_id)
                .map(PollMessage::Server)
        } else {
            crate::proto::h2::client::Peer::convert_poll_message(pseudo, fields, stream_id)
                .map(PollMessage::Client)
        }
    }

    pub fn ensure_can_open(&self, id: StreamId, mode: Open) -> Result<(), RecvError> {
        if self.is_server() {
            if mode.is_push_promise() || !id.is_client_initiated() {
                proto_err!(conn: "cannot open stream {:?} - not client initiated", id);
                return Err(RecvError::Connection(Reason::PROTOCOL_ERROR));
            }

            Ok(())
        } else {
            if !mode.is_push_promise() || !id.is_server_initiated() {
                proto_err!(conn: "cannot open stream {:?} - not server initiated", id);
                return Err(RecvError::Connection(Reason::PROTOCOL_ERROR));
            }

            Ok(())
        }
    }
}
