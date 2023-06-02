use futures_timer::Delay;
use futures_util::{future, FutureExt};

use gloo_net::websocket::{futures::WebSocket, Message, WebSocketError};
use smoldot::libp2p::multiaddr::{Multiaddr, ProtocolRef};

use futures_util::stream::{SplitSink, SplitStream, StreamExt};

use futures_util::lock::Mutex;
use std::sync::Arc;
use std::{
    io::IoSlice,
    net::{IpAddr, SocketAddr},
};

use tokio::sync::{mpsc, oneshot};

#[derive(Clone)]
pub struct Platform {}

impl Platform {
    pub const fn new() -> Self {
        Self {}
    }
}

impl smoldot_light::platform::PlatformRef for Platform {
    type Delay = future::BoxFuture<'static, ()>;

    // No-op yielding.
    type Yield = future::Ready<()>;

    type Instant = std::time::Instant;

    type Connection = std::convert::Infallible;

    type Stream = ConnectionSocket;

    type ConnectFuture = future::BoxFuture<'static, Result<ConnectionSocket, ConnectError>>;

    type StreamUpdateFuture<'a> = future::BoxFuture<'a, ()>;

    type NextSubstreamFuture<'a> = future::Pending<()>;

    fn now_from_unix_epoch(&self) -> std::time::Duration {
        std::time::UNIX_EPOCH
            .elapsed()
            .expect("Invalid systime cannot be configured earlier than `UNIX_EPOCH`")
    }

    fn now(&self) -> Self::Instant {
        std::time::Instant::now()
    }

    fn sleep(&self, duration: std::time::Duration) -> Self::Delay {
        futures_timer::Delay::new(duration).boxed()
    }

    fn sleep_until(&self, when: Self::Instant) -> Self::Delay {
        self.sleep(when.saturating_duration_since(self.now()))
    }

    fn spawn_task(
        &self,
        task_name: std::borrow::Cow<str>,
        task: futures_util::future::BoxFuture<'static, ()>,
    ) {
        println!("Spawning {task_name}");
        wasm_bindgen_futures::spawn_local(task)
    }

    fn client_name(&self) -> std::borrow::Cow<str> {
        "subxt".into()
    }

    fn client_version(&self) -> std::borrow::Cow<str> {
        env!("CARGO_PKG_VERSION").into()
    }

    fn yield_after_cpu_intensive(&self) -> Self::Yield {
        future::ready(())
    }

    fn connect(&self, url: &str) -> Self::ConnectFuture {
        Box::pin(async move {
            let multiaddr = url.parse::<Multiaddr>().map_err(|_| ConnectError {
                message: format!("Address {url} is not a valid multiaddress"),
                is_bad_addr: true,
            })?;

            // First two protocals must be valid, the third one is optional.
            let mut proto_iter = multiaddr.iter().fuse();

            let addr = match (
                proto_iter.next().ok_or(ConnectError {
                    message: format!("Unknown protocol combination"),
                    is_bad_addr: true,
                })?,
                proto_iter.next().ok_or(ConnectError {
                    message: format!("Unknown protocol combination"),
                    is_bad_addr: true,
                })?,
                proto_iter.next(),
            ) {
                (ProtocolRef::Ip4(ip), ProtocolRef::Tcp(port), None) => {
                    SocketAddr::new(IpAddr::V4((ip).into()), port)
                }
                (ProtocolRef::Ip6(ip), ProtocolRef::Tcp(port), None) => {
                    SocketAddr::new(IpAddr::V6((ip).into()), port)
                }
                (ProtocolRef::Ip4(ip), ProtocolRef::Tcp(port), Some(ProtocolRef::Ws)) => {
                    SocketAddr::new(IpAddr::V4((ip).into()), port)
                }
                (ProtocolRef::Ip6(ip), ProtocolRef::Tcp(port), Some(ProtocolRef::Ws)) => {
                    SocketAddr::new(IpAddr::V6((ip).into()), port)
                }
                // // TODO: we don't care about the differences between Dns, Dns4, and Dns6
                // (
                //     ProtocolRef::Dns(addr) | ProtocolRef::Dns4(addr) | ProtocolRef::Dns6(addr),
                //     ProtocolRef::Tcp(port),
                //     None,
                // ) => (either::Right((addr.to_string(), *port)), None),
                // (
                //     ProtocolRef::Dns(addr) | ProtocolRef::Dns4(addr) | ProtocolRef::Dns6(addr),
                //     ProtocolRef::Tcp(port),
                //     Some(ProtocolRef::Ws),
                // ) => (
                //     either::Right((addr.to_string(), *port)),
                //     Some(format!("{}:{}", addr, *port)),
                // ),
                _ => {
                    return Err(ConnectError {
                        is_bad_addr: true,
                        message: "Unknown protocols combination".to_string(),
                    })
                }
            };

            // TODO: use `addr` instead.
            let websocket = WebSocket::open(url.as_ref()).map_err(|e| ConnectError {
                is_bad_addr: false,
                message: "Cannot stablish WebSocket connection".to_string(),
            })?;

            let (to_sender, from_sender) = mpsc::channel(1024);
            let (to_receiver, from_receiver) = mpsc::channel(1024);

            // Note: WebSocket is not `Send`, work around that with a spawned task.

            wasm_bindgen_futures::spawn_local(async move {
                let backend_event = tokio_stream::wrappers::ReceiverStream::new(from_sender);
                
                let rpc_responses_event =
                    futures_util::stream::unfold(rpc_responses, |mut rpc_responses| async {
                        rpc_responses
                            .next()
                            .await
                            .map(|result| (result, rpc_responses))
                    });

                tokio::pin!(backend_event, rpc_responses_event);

                let mut backend_event_fut = backend_event.next();
                let mut rpc_responses_fut = rpc_responses_event.next();

                loop {
                    match future::select(backend_event_fut, rpc_responses_fut).await {
                        // Message received from the backend: user registered.
                        Either::Left((backend_value, previous_fut)) => {
                            let Some(message) = backend_value else {
                                tracing::trace!(target: LOG_TARGET, "Frontend channel closed");
                                break;
                            };
                            tracing::trace!(
                                target: LOG_TARGET,
                                "Received register message {:?}",
                                message
                            );

                            self.handle_register(message).await;

                            backend_event_fut = backend_event.next();
                            rpc_responses_fut = previous_fut;
                        }
                        // Message received from rpc handler: lightclient response.
                        Either::Right((response, previous_fut)) => {
                            // Smoldot returns `None` if the chain has been removed (which subxt does not remove).
                            let Some(response) = response else {
                                tracing::trace!(target: LOG_TARGET, "Smoldot RPC responses channel closed");
                                break;
                            };
                            tracing::trace!(
                                target: LOG_TARGET,
                                "Received smoldot RPC result {:?}",
                                response
                            );

                            self.handle_rpc_response(response).await;

                            // Advance backend, save frontend.
                            backend_event_fut = previous_fut;
                            rpc_responses_fut = rpc_responses_event.next();
                        }
                    }
                }
            });

            let (sender, receiver) = websocket.split();
            Ok(ConnectionSocket { sender, receiver })
        })
    }

    fn open_out_substream(&self, _connection: &mut Self::Connection) {
        // Called from MultiStream connections that are not supported.
    }

    fn next_substream<'a>(
        &self,
        connection: &'a mut Self::Connection,
    ) -> Self::NextSubstreamFuture<'a> {
        // Called from MultiStream connections that are not supported.
    }

    fn update_stream<'a>(&self, stream: &'a mut Self::Stream) -> Self::StreamUpdateFuture<'a> {
        todo!()
    }

    fn read_buffer<'a>(
        &self,
        stream: &'a mut Self::Stream,
    ) -> smoldot_light::platform::ReadBuffer<'a> {
        todo!()
    }

    fn advance_read_cursor(&self, stream: &mut Self::Stream, bytes: usize) {
        todo!()
    }

    fn writable_bytes(&self, stream: &mut Self::Stream) -> usize {
        todo!()
    }

    fn send(&self, stream: &mut Self::Stream, data: &[u8]) {
        todo!()
    }

    fn close_send(&self, stream: &mut Self::Stream) {
        todo!()
    }
}

/// Error potentially returned by [`PlatformRef::connect`].
pub struct ConnectError {
    /// Human-readable error message.
    pub message: String,
    /// `true` if the error is caused by the address to connect to being forbidden or unsupported.
    pub is_bad_addr: bool,
}

pub struct ConnectionSocket {
    sender: SplitSink<WebSocket, Message>,
    receiver: SSplitStream<WebSocket>,
}
