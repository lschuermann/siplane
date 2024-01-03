use eventsource_client::SSE;
use futures::{SinkExt, StreamExt};
use log::{debug, error, info, trace, warn};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::connect_async;
use tungstenite::Message;
use uuid::Uuid;

// TODO: share between server and proxy
mod listener_sse_api {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Listening {
        pub public_hostname: String,
        pub public_port: u16,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct NewConnection {
        pub client_ip: String,
        pub client_port: u16,
        pub remote_ip: String,
        pub remote_port: u16,
        pub rendezvous_endpoint: String,
        pub accept_token: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    #[serde(tag = "type")]
    pub enum SSEMessage {
        Listening(Listening),
        NewConnection(NewConnection),
    }
}

enum RendezvousListenerCmd {
    Shutdown,
}

enum RendezvousConnectionCmd {
    Shutdown,
}

pub struct RendezvousProxyState {
    client_id: Uuid,
    server_base_url: String,
    target_addr: SocketAddr,
    auth_token: String,
    sse_keepalive_timeout: Duration,
    sse_reconnect_wait: Duration,
    connections: Mutex<(
        u64,
        HashMap<
            u64,
            (
                tokio::sync::mpsc::Sender<RendezvousConnectionCmd>,
                tokio::task::JoinHandle<Result<(), RendezvousProxyError>>,
            ),
        >,
    )>,
    public_addr: Mutex<Option<(String, u16)>>,
    public_addr_notify: tokio::sync::Notify,
}

pub struct RendezvousProxy {
    state: Arc<RendezvousProxyState>,
    listener_cmd_tx: tokio::sync::mpsc::Sender<RendezvousListenerCmd>,
    listener_join_handle: tokio::task::JoinHandle<Result<(), RendezvousProxyError>>,
}

impl RendezvousProxy {
    pub async fn start(
        client_id: Uuid,
        server_base_url: String,
        target_addr: SocketAddr,
        auth_token: String,
        sse_keepalive_timeout: Duration,
        sse_reconnect_wait: Duration,
    ) -> Self {
        let state = Arc::new(RendezvousProxyState {
            client_id,
            server_base_url,
            target_addr,
            auth_token,
            sse_keepalive_timeout,
            sse_reconnect_wait,
            connections: Mutex::new((0, HashMap::new())),
            public_addr: Mutex::new(None),
            public_addr_notify: tokio::sync::Notify::new(),
        });

        let task_state = state.clone();
        let (listener_cmd_tx, listener_cmd_rx) = tokio::sync::mpsc::channel(1);
        let listener_join_handle =
            tokio::task::spawn(async move { sse_loop(task_state, listener_cmd_rx).await });

        RendezvousProxy {
            state,
            listener_cmd_tx,
            listener_join_handle,
        }
    }

    pub async fn public_addr(&self, timeout: Duration) -> Option<(String, u16)> {
        let start = std::time::Instant::now();

        loop {
            let public_addr_lg = self.state.public_addr.lock().await;

            if let Some(ref public_addr) = *public_addr_lg {
                return Some(public_addr.clone());
            }

            let time_since_start = Instant::now().duration_since(start);
            if time_since_start >= timeout {
                return None;
            }

            // Request a notification before releasing the lock, to avoid racing
            // with another thread setting the variable and then notifying:
            let fut = self.state.public_addr_notify.notified();
            std::mem::drop(public_addr_lg);

            #[rustfmt::skip]
	    tokio::select! {
		_ = fut => (),
		_ = tokio::time::sleep(
		    timeout.checked_sub(time_since_start)
			.unwrap_or(Duration::ZERO)
		) => (),
	    }
        }
    }

    pub async fn shutdown(self) -> Result<(), RendezvousProxyError> {
        // Shut down the listener first:
        info!("Requesting rendezvous listener to shut down...");
        self.listener_cmd_tx
            .send(RendezvousListenerCmd::Shutdown)
            .await
            .expect("Rendezvous listener task has quit before receiving shutdown signal!");
        self.listener_join_handle.await.unwrap()?;

        // Then, shut down all open connections:
        let (_, ref mut connections) = *self.state.connections.lock().await;
        for (connection_id, (connection_cmd_tx, join_handle)) in connections.drain() {
            // It's fine (and expected) for the connection tasks to exit
            // whenever the connection was closed by either client or server, or
            // when an error occurred:
            let _ = connection_cmd_tx
                .send(RendezvousConnectionCmd::Shutdown)
                .await;

            // For now, we also ignore invididual connection errors (simply log them):
            if let Err(e) = join_handle.await.unwrap() {
                warn!(
                    "Connection {} has experienced an error: {:?}",
                    connection_id, e
                );
            }
        }

        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum RendezvousProxyError {
    #[error("the SSE request could not be constructed")]
    SSEClientBuilderError(eventsource_client::Error),

    #[error("the websocket client could not be constructed")]
    WSRequestBuilderError(tungstenite::Error),

    #[error("the websocket client could not be constructed")]
    WSConnectionError(tungstenite::Error),

    #[error("connecting to the requested TCP service endpoint failed")]
    TCPServiceConnectionError(std::io::Error),

    #[error("reading from the TCP service stream failed")]
    TCPServiceReadError(std::io::Error),

    #[error("writing to the TCP service stream failed")]
    TCPServiceWriteError(std::io::Error),

    #[error("reading from the rendezvous server websocket connection failed")]
    RendezvousServerWSReadError(tungstenite::Error),

    #[error("writing to the rendezvous server websocket connection failed")]
    RendezvousServerWSWriteError(tungstenite::Error),
}

async fn sse_loop(
    state: Arc<RendezvousProxyState>,
    mut cmd_rx: tokio::sync::mpsc::Receiver<RendezvousListenerCmd>,
) -> Result<(), RendezvousProxyError> {
    use eventsource_client::Client;
    use futures::TryStreamExt;

    // Whether the listener command channel has been dropped. We
    // detect this case and log a warning, but otherwise continue
    // operating. This may happen if the caller of ::start() drops the
    // returned RendezvousProxy. The caller should retain this struct
    // and call shutdown eventually.
    let mut cmd_channel_dropped = false;

    // Outer loop, to reconnect in case the SSE stream dies.
    loop {
        info!("Connecting to rendezvous server...");

        // TODO: inject token as Bearer auth
        let client = eventsource_client::ClientBuilder::for_url(&format!(
            "{}/listen/{}",
            state.server_base_url, state.client_id,
        ))
        .and_then(|c| c.header("Authorization", &format!("Bearer {}", state.auth_token)))
        .map_err(RendezvousProxyError::SSEClientBuilderError)?
        .build();

        // Initialize the keepalive timeout by tracking the last keepalive
        // message in an `Instant`:
        let mut last_message = Instant::now();

        // Stream individual SSE messages from the client:
        let mut stream = Box::pin(client.stream());

        // Whether to attempt a reconnect upon an error / shutdown request:
        let mut reconnect = true;

        // Inner loop to handle SSE messages.
        loop {
            #[rustfmt::skip]
            tokio::select! {
		listener_cmd_opt = cmd_rx.recv(), if !cmd_channel_dropped => {
		    match listener_cmd_opt {
			Some(RendezvousListenerCmd::Shutdown) => {
			    reconnect = false;
			    break;
			},

			None => {
			    warn!(
				"Listener command channel closed. Did \
				 you drop the `RendezvousProxy` \
				 struct?"
			    );
			    cmd_channel_dropped = true;
			}
		    }
		}

                sse_stream_element = stream.try_next() => {
                    match sse_stream_element {
                        Ok(Some(SSE::Event(ev))) => {
			    trace!("Received SSE event: {:?}", ev);
                            handle_sse_event(ev, &state).await;
                            last_message = Instant::now();
                        }

                        Ok(Some(SSE::Comment(c))) => {
                            // Do nothing. We use comments for keep-alive
			    // messages only.
			    trace!("Received keep-alive comment: {:?}", c);
                            last_message = Instant::now();
                        }

                        Ok(None) => {
                            // Ignore empty stream elements.
                        },

                        Err(e) => {
                            warn!("Error while processing SSE stream, \
				    terminating connection: {:?}", e);
                            break
                        }
                    }
                }

                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    // Check whether the keepalive timeout has expired:
                    if Instant::now().duration_since(last_message)
			> state.sse_keepalive_timeout
		    {
                        warn!(
                            "No coordinator message for {} sec, terminating \
			     connection.", state.sse_keepalive_timeout.as_secs()
                        );
                        break
                    }
                }
            }
        }

        if reconnect {
            warn!(
                "EventSource connection closed, attempting to reconnect after {} \
		 sec...",
                state.sse_reconnect_wait.as_secs()
            );
            tokio::time::sleep(state.sse_reconnect_wait).await;
        } else {
            info!("Shut down listener SSE connection.");
            break;
        }
    }

    Ok(())
}

async fn handle_sse_event(ev: eventsource_client::Event, state: &Arc<RendezvousProxyState>) {
    use listener_sse_api::SSEMessage;

    match ev.event_type.as_str() {
        "message" => match serde_json::from_str::<SSEMessage>(&ev.data) {
            Ok(SSEMessage::Listening(l)) => {
                info!(
                    "Listening on rendezvous server at: {}:{}",
                    l.public_hostname, l.public_port,
                );
                let mut public_addr_lg = state.public_addr.lock().await;
                *public_addr_lg = Some((l.public_hostname, l.public_port));
                state.public_addr_notify.notify_waiters();
                std::mem::drop(public_addr_lg);
            }

            Ok(SSEMessage::NewConnection(nc)) => {
                debug!(
                    "Incoming connection request from rendezvous server: \
			  {:?}",
                    nc
                );
                handle_new_connection_event(nc, state).await;
            }

            Err(e) => {
                warn!("Unable to parse SSE message \"{}\": {:?}", ev.data, e);
            }
        },

        _ => warn!("Received unknown SSE event type {}!", ev.event_type),
    }
}

async fn handle_new_connection_event(
    new_connection: listener_sse_api::NewConnection,
    state: &Arc<RendezvousProxyState>,
) {
    // We lock state here across the invocation of tokio::task::spawn. This is
    // fine and cannot deadlock, as a lock attempt in this or another task will
    // eventually resume execution in this task. We need to hold this lock to
    // increment the connection_cnt and insert the connection's task join handle
    // consistently.
    let (ref mut connections_cnt, ref mut connections) = *state.connections.lock().await;

    let connection_id: u64 = *connections_cnt;
    *connections_cnt = connections_cnt
        .checked_add(1)
        .expect("Invariant violated: overflow in connections_cnt");

    info!(
        "Opening connection {} to {:?}",
        connection_id, state.target_addr
    );

    let (connection_cmd_tx, connection_cmd_rx) = tokio::sync::mpsc::channel(1);

    let task_state = state.clone();
    let join_handle = tokio::task::spawn(async move {
        let res =
            connection_task(connection_id, new_connection, connection_cmd_rx, task_state).await;
        if let Err(ref e) = res {
            warn!("Connection {} closed with error: {:?}", connection_id, e);
        } else {
            info!("Connection {} closed.", connection_id);
        }
        res
    });

    connections.insert(connection_id, (connection_cmd_tx, join_handle));
}

async fn connection_task(
    connection_id: u64,
    new_connection: listener_sse_api::NewConnection,
    mut connection_cmd_rx: tokio::sync::mpsc::Receiver<RendezvousConnectionCmd>,
    state: Arc<RendezvousProxyState>,
) -> Result<(), RendezvousProxyError> {
    // Connect to the Tcp endpoint. We do this first, as we can be
    // more confident that the WebSocket connection will succeed
    // (given that we just received an SSE message indicating that
    // a connection has been accepted). If the TCP connection
    // fails, we don't waste resources connecting to the WebSocket
    // endpoint first:
    debug!(
        "Connection {}: dialing TcpStream to {:?}",
        connection_id, state.target_addr
    );
    let mut tcp_stream = TcpStream::connect(&state.target_addr)
        .await
        .map_err(RendezvousProxyError::TCPServiceConnectionError)?;

    // Connect to the websocket endpoint:
    debug!(
        "Connection {}: building WebSocket request to {}",
        connection_id, &new_connection.rendezvous_endpoint
    );

    // Compare logic of
    // https://github.com/snapview/tungstenite-rs/blob/2ee05d10803d95ad48b3ad03d9d9a03164060e76/src/client.rs#L219-L248
    let ws_request = {
        let ws_uri = new_connection
            .rendezvous_endpoint
            .parse::<http::Uri>()
            .map_err(|e| {
                RendezvousProxyError::WSRequestBuilderError(tungstenite::error::Error::from(e))
            })?;

        let ws_authority = ws_uri
            .authority()
            .ok_or(RendezvousProxyError::WSRequestBuilderError(
                tungstenite::error::Error::Url(tungstenite::error::UrlError::NoHostName),
            ))?
            .as_str();
        let ws_host = ws_authority
            .find('@')
            .map(|idx| ws_authority.split_at(idx + 1).1)
            .unwrap_or_else(|| ws_authority);

        tungstenite::handshake::client::Request::builder()
            .method("GET")
            .header("Host", ws_host)
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tungstenite::handshake::client::generate_key(),
            )
            .header(
                "Authorization",
                format!("Bearer {}", new_connection.accept_token),
            )
            .uri(&new_connection.rendezvous_endpoint)
            .body(())
            .map_err(|e| RendezvousProxyError::WSRequestBuilderError(tungstenite::Error::from(e)))?
    };

    debug!(
        "Connection {}: Connecting to rendezvous endpoint {}",
        connection_id, &new_connection.rendezvous_endpoint,
    );
    let (mut ws_client, _) = connect_async(ws_request)
        .await
        .map_err(RendezvousProxyError::WSConnectionError)?;
    debug!(
        "Connection {}: connected to WebSocket endpoint!",
        connection_id
    );

    // Temporary buffer space for receiving from the TCP socket:
    let mut buf = vec![0; 64 * 1024];

    let res = loop {
        #[rustfmt::skip]
	let res: Result<bool, RendezvousProxyError> = tokio::select! {
	    cmd = connection_cmd_rx.recv() => {
		match cmd {
		    Some(RendezvousConnectionCmd::Shutdown) => {
			// Request immediate shutdown:
			Ok(false)
		    }

		    None => {
			panic!(
			    "Connection {}: Invariant violated: connection \
			     cmd channel closed prematurely!",
			    connection_id,
			);
		    }
		}
	    }

	    size_res = tcp_stream.read(&mut buf) => {
		match size_res {
		    Ok(size) => {
			ws_client.send(
			    Message::Binary(buf[..size].to_vec())
			)
			    .await
			    .map_err(RendezvousProxyError::RendezvousServerWSWriteError)
			    .map(|_| /* Continue */ true)
		    },

		    Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
			debug!(
			    "Connection {}: TCP stream terminated \
			     connection", connection_id,
			);

			// Request immediate shutdown:
			Ok(false)
		    }

		    Err(e) => {
			error!(
			    "Connection {}: Reading from TCP stream failed: \
			     {:?}", connection_id, e,
			);
			Err(RendezvousProxyError::TCPServiceReadError(e))
		    }
		}
	    }

	    message = ws_client.next() => {
		match message {
		    Some(Ok(Message::Binary(vec))) => {
			tcp_stream.write_all(&vec)
			    .await
			    .map_err(RendezvousProxyError::TCPServiceWriteError)
			    .map(|_| /* Continue */ true)
		    },

		    // We handle pings, but don't send out pings
		    // ourselves. Thus we don't need to handle pongs.
		    Some(Ok(Message::Ping(vec))) => {
			ws_client.send(
			    Message::Ping(vec)
			)
			    .await
			    .map_err(RendezvousProxyError::RendezvousServerWSWriteError)
			    .map(|_| /* Continue */ true)
		    },

		    Some(Ok(Message::Close(_close_frame))) => {
			// Request immediate shutdown:
			Ok(false)
		    }


		    Some(Ok(msg)) => {
			warn!(
			    "Connection {}: Received unexpected websocket \
			     message: {:?}", connection_id, msg
			);

			// Continue, this should be harmless:
			Ok(true)
		    }

		    Some(Err(e)) => {
			warn!(
			    "Connection {}: Error during websocket receive: \
			     {:?}", connection_id, e
			);
			Err(RendezvousProxyError::RendezvousServerWSReadError(e))
		    }

		    None => {
			warn!(
			    "Connection {}: Websocket connection closed \
			     unexpectedly",
			    connection_id,
			);
			Ok(false)
		    }
		}
	    }
	};

        // Abort the loop if requested, or an error occurred.
        match res {
            Ok(true) => (),
            Ok(false) => break Ok(()),
            Err(e) => break Err(e),
        }
    };

    // The TcpStream is closed on drop. (Attempt to) close the
    // WebSocket explicitly:
    if let Err(e) = ws_client.close(None).await {
        warn!(
            "Connection {}: error while attempting to close the \
	       WebSocket connection: {:?}",
            connection_id, e
        );
    } else {
        debug!("Connection {}: closed WebSocket connection.", connection_id);
    }

    res
}
