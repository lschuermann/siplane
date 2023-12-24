use log::{warn, debug, info, error};
use anyhow::{Result, Context};
use serde::{Serialize, Deserialize};
use tokio_seqpacket::{UnixSeqpacketListener, UnixSeqpacket};
use crate::connector;
use std::path::Path;
use tokio::net::UnixDatagram;
use tokio::sync::{RwLock, Mutex};
use tokio::task::JoinHandle;
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
#[serde(tag = "type")]
pub enum ControlReq {
    Ping,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
#[serde(tag = "type")]
pub enum ControlResp {
    Pong,
    RequestError {
	message: String,
    },
}

#[derive(Debug, Clone)]
enum ControlSocketTaskCommand {
    Shutdown,
}

struct ControlSocketState {
    client: Option<UnixSeqpacket>,
}

pub struct ControlSocket<R: connector::Runner> {
    task_handle: JoinHandle<Result<()>>,
    task_cmd_chan: tokio::sync::mpsc::Sender<ControlSocketTaskCommand>,
    state: Arc<RwLock<ControlSocketState>>,
    _runner: Arc<R>,
}

impl<R: connector::Runner> ControlSocket<R> {
    async fn handle_request(req: ControlReq, runner: &R, state: &mut ControlSocketState) -> ControlResp {
	match req {
	    ControlReq::Ping => {
		debug!("Received ping request!");
		ControlResp::Pong
	    },
	}
    }

    pub async fn new_unix_seqpacket(addr: &Path, runner: Arc<R>) -> Result<Self> {
	let mut server_socket: UnixSeqpacketListener =
	    UnixSeqpacketListener::bind(addr)
	    .with_context(|| format!("Binding to UNIX socket at \"{:?}\"", addr))?;
	info!("Opened control socket UNIX SeqPacket listener on \"{:?}\"", addr);

	let state = Arc::new(RwLock::new(ControlSocketState {
	    client: None,
	}));

	let (task_cmd_chan_tx, mut task_cmd_chan_rx) = tokio::sync::mpsc::channel(1);

	let task_state = state.clone();
	let task_runner = runner.clone();

	let task_handle = tokio::spawn(async move {
	    const RECV_RSV: usize = 1 * 1024 * 1024;

	    let state = task_state;
	    let runner = task_runner;

	    let mut shutdown_requested = false;
	    let mut recv_buf = vec![0; RECV_RSV];

	    while !shutdown_requested {
		// Accept new connections. We only handle one
		// connection at any point in time:
		let socket_res = tokio::select! {
		    accept_res = server_socket.accept() => {
			Ok(accept_res
			   .context("Accepting new control socket connection.").unwrap())
		    }

		    cmd_res = task_cmd_chan_rx.recv() => {
			error!("Received task command!");
			match cmd_res {
			    Some(cmd) => Err(cmd),
			    None => {
				panic!("Control socket command channel TX dropped!");
			    },
			}
		    }
		};

		let socket = match socket_res {
		    Ok(socket) => socket,
		    Err(ControlSocketTaskCommand::Shutdown) => {
			shutdown_requested = true;
			continue;
		    }
		};

		// Place the socket reference into the shared socket state:
		{
		    let mut sock_state = state.write().await;
		    sock_state.client = Some(socket);
		}

		loop {
		    let res = {
			// Acquire a read-lock over the shared socket
			// state. This ensures that we can send messages while
			// receiving:
			error!("Loop!");
			let sock_state = state.read().await;
			error!("Loop lock!");
			let socket = sock_state.client.as_ref().expect("Invariant violated: client socket removed while reading!");

			// Receive datagram. We impose an upper limit of 1MB on
			// datagrams, so a single .recv() call should be
			// sufficient:
			tokio::select! {
			    recv_res = socket.recv(&mut recv_buf) => {
				match recv_res {
				    Ok(size) => Ok(size),
				    // TODO: replace with error
				    Err(e) => panic!("Error occurred while receiving from control socket: {:?}", e),
				}
			    }

			    cmd_res = task_cmd_chan_rx.recv() => {
				error!("Received task command inner!");
				match cmd_res {
				    Some(cmd) => Err(cmd),
				    None => {
					panic!("Control socket command channel TX dropped!");
				    },
				}
			    }
			}
		    };

		    // Handle either an incoming request or a
		    // command. Importantly, we currently don't hold the
		    // state lock.
		    match res {
			Ok(size) => {
			    // For handling the request, acquire a write lock:
			    let mut sock_state = state.write().await;

			    // Attept to decode the datagram. If this fails, send a
			    // RequestError response containing the error
			    // message. Otherwise, pass the request onto the handle
			    // function:
			    debug!("Trying to decode {:?}", &recv_buf[..size]);
			    let resp =
				match serde_json::from_slice(&recv_buf[..size]) {
				    Ok(req) => Self::handle_request(req, &runner, &mut sock_state).await,
				    Err(e) => ControlResp::RequestError {
					message: format!("{:?}", e),
				    },
				};

			    let socket = sock_state.client.as_ref().expect("Invariant violated: client socket removed while reading!");
			    if let Err(e) = socket.send(&serde_json::to_vec(&resp).expect("Failed to encode control socket response as JSON")).await {
				warn!("Error while sending answer to control socket request, ignoring: {:?}", e);
			    }
			}
			Err(ControlSocketTaskCommand::Shutdown) => {
			    shutdown_requested = true;
			    break;
			}
		    }
		}
	    }

	    Ok(())
	});

	Ok(ControlSocket {
	    task_handle,
	    state,
	    task_cmd_chan: task_cmd_chan_tx,
	    _runner: runner,
	})
    }

    pub async fn shutdown(self) -> Result<()> {
	log::info!("Requesting shutdown.");
	// First, request shutdown of the task:
	self.task_cmd_chan.send(ControlSocketTaskCommand::Shutdown).await
	    .with_context(|| format!("Requesting shutdown of the control socket request handler"))?;

	// Then, try to join it:
	self.task_handle.await.unwrap();

	// Finally, so some cleanup.
	let mut state = self.state.write().await;

	// Joining the task implicitly drops the `UnixSeqpacketListener` to
	// close the FD, such that we can unmount the parent file
	// systems. However, the client connection may still be open. Shut that
	// down:
	state.client.take().map(|c| c.shutdown(std::net::Shutdown::Both))
	    .transpose()
	    .context("Shutting down control socket UNIX SeqPacket connection.")?;

	// The remainding cleanup happens when self is dropped.
	Ok(())
    }
}
