pub mod types;
mod utils;

use std::io::Cursor;
use std::time::Duration;

use anyhow::{anyhow, Result};
use async_compression::tokio::bufread::ZlibDecoder;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::spawn;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{interval, Instant};
use tokio_tungstenite::tungstenite::protocol::Message;

use self::types::{LeapEdgeAuthParams, OpCodes, SocketHello, SocketMessage};
use self::utils::connect;

const HOP_LEAP_EDGE_URL: &str = "wss://leap.hop.io/ws?encoding=json&compression=zlib";
const HOP_LEAP_EDGE_PROJECT_ID: &str = "project_MzA0MDgwOTQ2MDEwODQ5NzQ";

#[derive(Debug, Default)]
pub struct WebsocketClient {
    auth: Option<LeapEdgeAuthParams>,
    thread: Option<JoinHandle<()>>,
    channels: Option<SocketChannels>,
    last_heartbeat_acknowledged: bool,
    heartbeat_instants: (Option<Instant>, Option<Instant>),
}

#[derive(Debug)]
pub struct SocketChannels {
    send: mpsc::Sender<String>,
    recv: mpsc::Receiver<String>,
}

impl WebsocketClient {
    pub fn new() -> Self {
        let last_heartbeat_acknowledged = true;

        Self {
            last_heartbeat_acknowledged,
            ..Default::default()
        }
    }

    /// Called from login
    pub fn update_token(&mut self, token: String) {
        self.auth = Some(LeapEdgeAuthParams {
            project_id: HOP_LEAP_EDGE_PROJECT_ID.to_string(),
            token,
        });
    }

    pub async fn connect(mut self) -> Result<Self> {
        let (sender_outbound, mut receiver_outbound) = mpsc::channel::<String>(1);
        let (sender_inbound, receiver_inbound) = mpsc::channel::<String>(1);

        self.channels = Some(SocketChannels {
            send: sender_outbound.clone(),
            recv: receiver_inbound,
        });

        let socket_auth = self.auth.clone();

        // start massive thread to get messages / deliver messages
        let thread = spawn(async move {
            let client = connect().await.expect("Failed to connect to websocket");

            let (mut sender, mut receiver) = client.split();

            // the first message has to be server hello so lets wait for it
            let hello = receiver
                .next()
                .await
                .expect("Error reading from socket")
                .expect("Error reading from socket");

            let hello: SocketMessage<SocketHello> = Self::parse_message(hello).await;

            // it is safe to unwrap since first message **has** to be hello
            let htb = hello.d.unwrap().heartbeat_interval;

            log::debug!("Heartbeat interval: {}ms", htb);

            let mut interval = interval(Duration::from_millis(htb));

            // skip first htb
            interval.tick().await;

            sender_outbound
                .clone()
                .send(
                    serde_json::to_string(&SocketMessage {
                        op: OpCodes::Identify,
                        d: Some(socket_auth),
                    })
                    .unwrap(),
                )
                .await
                .expect("Failed to send identify message");

            loop {
                tokio::select! {
                    // gateway receiver
                    message = receiver.next() => {
                        match message {
                            Some(recieved) => match recieved {
                                Ok(message) => match Self::parse_message::<SocketMessage<Value>>(message).await {
                                    SocketMessage { op: OpCodes::HeartbeatAck, d: _ } => {
                                        self.last_heartbeat_acknowledged = true;

                                        // add current heartbeat instant to list of heartbeat instants
                                        self.heartbeat_instants.1 = Some(Instant::now());

                                        log::debug!("Heartbeat acknowledged, latency: {:?}", self.heartbeat_instants.1.unwrap().duration_since(self.heartbeat_instants.0.unwrap()));
                                    }

                                    SocketMessage { op: OpCodes::Heartbeat, d: tag } => {
                                        match sender.send(serde_json::to_string(&SocketMessage {
                                            op: OpCodes::Heartbeat,
                                            d: tag,
                                        }).unwrap().into()).await {
                                            Ok(_) => {
                                                log::debug!("Responded to tagged heartbeat");
                                            }

                                            Err(e) => {
                                                log::error!("Error sending heartbeat: {}", e)
                                            }
                                        }
                                    }

                                    SocketMessage { op: OpCodes::Dispatch, d: data } => {
                                        sender_inbound.send(serde_json::to_string(&data).unwrap()).await.ok();
                                    }

                                    // ignore other messages
                                    _ => {}
                                },

                                Err(err) => {
                                    // TODO: reconnect?
                                    log::error!("Error reading from socket: {}", err);
                                    sender_inbound.send("null".to_string()).await.unwrap();
                                }
                            },

                            // no idea why this would happen
                            None => {}
                        }
                    },

                    // internal rcv thread
                    internal = receiver_outbound.recv() => {
                        match internal {
                            Some(message) => {
                                log::debug!("Sending message: {}", message);

                                sender.send(message.into()).await.expect("Error sending message")
                            },
                            // no idea why this would happen
                            None => {}
                        }
                    },

                    // heartbeat sender
                    _ = interval.tick() => {
                        log::debug!("Sending heartbeat");

                        if !self.last_heartbeat_acknowledged {
                            log::debug!("Possible zombie connection: no heartbeat ack");
                            // TODO: reconnect?
                        } else {
                            self.last_heartbeat_acknowledged = false;
                        }

                        self.heartbeat_instants = (Some(Instant::now()), None);

                        let heartbeat: SocketMessage<()> = SocketMessage {
                            op: OpCodes::Heartbeat,
                            d: None,
                        };

                        sender.send(serde_json::to_string(&heartbeat).unwrap().into()).await.expect("Error sending heartbeat");
                    }
                }
            }
        });

        self.thread = Some(thread);

        Ok(self)
    }

    async fn parse_message<T>(message: Message) -> T
    where
        T: serde::de::DeserializeOwned + std::fmt::Debug,
    {
        match message {
            Message::Text(text) => {
                let message: T = serde_json::from_str(&text).expect("Failed to parse message");

                log::debug!("Received message: {:?}", message);

                message
            }

            Message::Binary(bin) => {
                let mut uncompressed = ZlibDecoder::new(Cursor::new(bin));
                let mut buff = vec![];
                uncompressed
                    .read_to_end(&mut buff)
                    .await
                    .expect("Failed to read message");

                let message: T = serde_json::from_slice(&buff).expect("Failed to deflate message");

                log::debug!("Received message: {:?}", message);

                message
            }

            Message::Close(close) => {
                panic!("Socket closed unexpectedly: {:?}", close);
            }

            _ => {
                log::debug!("Unknown message type: {:?}", message);
                panic!("received unexpected message type");
            }
        }
    }

    pub async fn recieve_message<T>(&mut self) -> Option<T>
    where
        T: serde::de::DeserializeOwned,
    {
        match self.channels {
            Some(ref mut channels) => channels
                .recv
                .recv()
                .await
                .map(|message| serde_json::from_str(&message).unwrap()),
            None => None,
        }
    }

    // TODO: remove when channels are implemented
    #[allow(dead_code)]
    pub async fn send_message<T>(&self, message: T) -> Result<()>
    where
        T: serde::Serialize,
    {
        match self.channels {
            Some(ref channels) => {
                let message = serde_json::to_string(&message).unwrap();

                channels.send.send(message).await?;

                Ok(())
            }
            None => Err(anyhow!("Not connected")),
        }
    }

    pub async fn close(&mut self) {
        if self.channels.is_some() {
            self.channels = None;
        }

        if let Some(thread) = self.thread.as_ref() {
            thread.abort();
            self.thread = None;
        }

        self.auth = None;
        self.heartbeat_instants = (None, None);
        self.last_heartbeat_acknowledged = true;
    }
}
