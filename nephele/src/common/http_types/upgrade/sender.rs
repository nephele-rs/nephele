use crate::common::http_types::upgrade::Connection;

#[derive(Debug)]
pub struct Sender {
    sender: cynthia::platform::channel::Sender<Connection>,
}

impl Sender {
    #[doc(hidden)]
    pub fn new(sender: cynthia::platform::channel::Sender<Connection>) -> Self {
        Self { sender }
    }

    pub async fn send(self, conn: Connection) {
        let _ = self.sender.send(conn).await;
    }
}
