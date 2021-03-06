use cynthia::future::swap::AsyncWrite;
use std::task::{Context, Poll};

use crate::proto::h2::codec::{RecvError, UserError};
use crate::proto::h2::error::Reason;
use crate::proto::h2::frame;
use crate::proto::h2::proto::*;

#[derive(Debug)]
pub(crate) struct Settings {
    local: Local,
    remote: Option<frame::Settings>,
}

#[derive(Debug)]
enum Local {
    ToSend(frame::Settings),
    WaitingAck(frame::Settings),
    Synced,
}

impl Settings {
    pub(crate) fn new(local: frame::Settings) -> Self {
        Settings {
            local: Local::WaitingAck(local),
            remote: None,
        }
    }

    pub(crate) fn recv_settings<T, B, C, P>(
        &mut self,
        frame: frame::Settings,
        codec: &mut Codec<T, B>,
        streams: &mut Streams<C, P>,
    ) -> Result<(), RecvError>
    where
        T: AsyncWrite + Unpin,
        B: Buf,
        C: Buf,
        P: Peer,
    {
        if frame.is_ack() {
            match &self.local {
                Local::WaitingAck(local) => {
                    tracing::debug!("received settings ACK; applying {:?}", local);

                    if let Some(max) = local.max_frame_size() {
                        codec.set_max_recv_frame_size(max as usize);
                    }

                    if let Some(max) = local.max_header_list_size() {
                        codec.set_max_recv_header_list_size(max as usize);
                    }

                    streams.apply_local_settings(local)?;
                    self.local = Local::Synced;
                    Ok(())
                }
                Local::ToSend(..) | Local::Synced => {
                    proto_err!(conn: "received unexpected settings ack");
                    Err(RecvError::Connection(Reason::PROTOCOL_ERROR))
                }
            }
        } else {
            assert!(self.remote.is_none());
            self.remote = Some(frame);
            Ok(())
        }
    }

    pub(crate) fn send_settings(&mut self, frame: frame::Settings) -> Result<(), UserError> {
        assert!(!frame.is_ack());
        match &self.local {
            Local::ToSend(..) | Local::WaitingAck(..) => Err(UserError::SendSettingsWhilePending),
            Local::Synced => {
                tracing::trace!("queue to send local settings: {:?}", frame);
                self.local = Local::ToSend(frame);
                Ok(())
            }
        }
    }

    pub(crate) fn poll_send<T, B, C, P>(
        &mut self,
        cx: &mut Context,
        dst: &mut Codec<T, B>,
        streams: &mut Streams<C, P>,
    ) -> Poll<Result<(), RecvError>>
    where
        T: AsyncWrite + Unpin,
        B: Buf,
        C: Buf,
        P: Peer,
    {
        if let Some(settings) = &self.remote {
            if !dst.poll_ready(cx)?.is_ready() {
                return Poll::Pending;
            }

            let frame = frame::Settings::ack();

            dst.buffer(frame.into()).expect("invalid settings frame");

            tracing::trace!("ACK sent; applying settings");

            if let Some(val) = settings.header_table_size() {
                dst.set_send_header_table_size(val as usize);
            }

            if let Some(val) = settings.max_frame_size() {
                dst.set_max_send_frame_size(val as usize);
            }

            streams.apply_remote_settings(settings)?;
        }

        self.remote = None;

        match &self.local {
            Local::ToSend(settings) => {
                if !dst.poll_ready(cx)?.is_ready() {
                    return Poll::Pending;
                }

                dst.buffer(settings.clone().into())
                    .expect("invalid settings frame");
                tracing::trace!("local settings sent; waiting for ack: {:?}", settings);

                self.local = Local::WaitingAck(settings.clone());
            }
            Local::WaitingAck(..) | Local::Synced => {}
        }

        Poll::Ready(Ok(()))
    }
}
