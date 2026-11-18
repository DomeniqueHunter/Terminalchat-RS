//! Websocket connection to the chat server.
//!
//! Outgoing messages are queued and sent at most one per second (the server drops clients that
//! flood it). Ping answers use the priority queue and jump ahead of everything else.

use std::collections::VecDeque;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{sleep_until, timeout, Instant};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::error::{Error, Result};
use crate::opcode;

pub const SEND_INTERVAL: Duration = Duration::from_secs(1);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq)]
pub struct ServerMessage {
    pub op: String,
    pub data: Value,
}

#[derive(Debug)]
pub enum Event {
    Message(ServerMessage),
    /// The connection ended; the text says why.
    Closed(String),
}

#[derive(Debug, Clone)]
pub struct Identity {
    pub account: String,
    pub ticket: String,
    pub character: String,
    pub client_name: String,
    pub client_version: String,
}

struct Outgoing {
    text: String,
    priority: bool,
}

pub struct Connection {
    tx: mpsc::UnboundedSender<Outgoing>,
    tasks: Vec<JoinHandle<()>>,
}

/// Splits `"OP {json}"` into the operation code and its payload.
pub fn parse_message(text: &str) -> Option<ServerMessage> {
    let op = text.get(..3)?;
    let rest = text.get(3..)?.trim_start();
    let data = if rest.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(rest).ok()?
    };
    Some(ServerMessage { op: op.to_string(), data })
}

pub fn format_message(op: &str, data: Option<&Value>) -> String {
    match data {
        Some(value) if !value.is_null() => format!("{op} {value}"),
        _ => op.to_string(),
    }
}

impl Connection {
    /// Connects, sends the `IDN` identification and starts the reader and sender tasks.
    /// The server answers with `IDN` (or `ERR`) through the returned event channel.
    pub async fn connect(url: &str, identity: &Identity) -> Result<(Connection, mpsc::UnboundedReceiver<Event>)> {
        let connecting = timeout(CONNECT_TIMEOUT, connect_async(url));
        let (socket, _) = connecting
            .await
            .map_err(|_| Error::Temporary("Could not connect to the chat server (timeout)".into()))?
            .map_err(|error| Error::Temporary(format!("Could not connect to the chat server: {error}")))?;
        let (mut sink, mut stream) = socket.split();

        let hello = json!({
            "method": "ticket",
            "ticket": identity.ticket,
            "account": identity.account,
            "character": identity.character,
            "cname": identity.client_name,
            "cversion": identity.client_version,
        });
        sink.send(Message::Text(format_message(opcode::IDENTIFY, Some(&hello)).into()))
            .await
            .map_err(|error| Error::Temporary(format!("Could not identify: {error}")))?;

        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (tx, mut rx) = mpsc::unbounded_channel::<Outgoing>();

        let reader_events = event_tx.clone();
        let reader = tokio::spawn(async move {
            let reason = loop {
                match stream.next().await {
                    Some(Ok(Message::Text(text))) => {
                        if let Some(message) = parse_message(text.as_str()) {
                            if reader_events.send(Event::Message(message)).is_err() {
                                return;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break "Connection closed".to_string(),
                    Some(Ok(_)) => {}
                    Some(Err(error)) => break format!("Connection error: {error}"),
                }
            };
            let _ = reader_events.send(Event::Closed(reason));
        });

        let sender = tokio::spawn(async move {
            let mut queue: VecDeque<String> = VecDeque::new();
            let mut next_allowed = Instant::now();
            loop {
                tokio::select! {
                    incoming = rx.recv() => {
                        let Some(item) = incoming else { return };
                        if item.priority {
                            queue.push_front(item.text);
                        } else {
                            queue.push_back(item.text);
                        }
                    }
                    _ = sleep_until(next_allowed), if !queue.is_empty() => {
                        if let Some(text) = queue.pop_front() {
                            if sink.send(Message::Text(text.into())).await.is_err() {
                                let _ = event_tx.send(Event::Closed("Could not send data to the server".into()));
                                return;
                            }
                            next_allowed = Instant::now() + SEND_INTERVAL;
                        }
                    }
                }
            }
        });

        Ok((Connection { tx, tasks: vec![reader, sender] }, event_rx))
    }

    pub fn send(&self, op: &str, data: Option<Value>) {
        let _ = self.tx.send(Outgoing { text: format_message(op, data.as_ref()), priority: false });
    }

    pub fn send_priority(&self, op: &str, data: Option<Value>) {
        let _ = self.tx.send(Outgoing { text: format_message(op, data.as_ref()), priority: true });
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_message_with_payload() {
        let message = parse_message(r#"MSG {"channel":"x","message":"hi"}"#).unwrap();
        assert_eq!(message.op, "MSG");
        assert_eq!(message.data["message"], "hi");
    }

    #[test]
    fn parses_message_without_payload() {
        let message = parse_message("PIN").unwrap();
        assert_eq!(message.op, "PIN");
        assert!(message.data.is_null());
    }

    #[test]
    fn rejects_broken_json() {
        assert!(parse_message("MSG {broken").is_none());
    }

    #[test]
    fn formats_messages() {
        assert_eq!(format_message("PIN", None), "PIN");
        assert_eq!(format_message("JCH", Some(&json!({"channel": "a"}))), r#"JCH {"channel":"a"}"#);
    }
}
