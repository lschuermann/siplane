use anyhow::{Context, Result};
use log::{debug, error, info, warn};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use uuid::Uuid;

use treadmill_rs::api::runner_puppet::{PuppetMsg, PuppetReq, RunnerMsg, RunnerResp};
use treadmill_rs::control_socket::Runner;

#[derive(Debug, Clone)]
enum ControlSocketTaskCommand {
    Shutdown,
}

// struct ControlSocketState {
//     // client: Option<Framed<TcpStream, LengthDelimitedCodec>>,
// }

pub struct TcpControlSocket<R: Runner> {
    _job_id: Uuid,
    task_handle: JoinHandle<Result<()>>,
    task_cmd_chan: tokio::sync::mpsc::Sender<ControlSocketTaskCommand>,
    // state: Arc<RwLock<ControlSocketState>>,
    _runner: Arc<R>,
}

impl<R: Runner> TcpControlSocket<R> {
    async fn handle_request(
        _request_id: u64,
        req: PuppetReq,
        job_id: Uuid,
        runner: &R,
        // _state: &mut ControlSocketState,
    ) -> RunnerResp {
        match req {
            PuppetReq::Ping => RunnerResp::PingResp,

            PuppetReq::SSHKeys => RunnerResp::SSHKeysResp {
                ssh_keys: runner.ssh_keys(job_id).await.unwrap_or_else(|| vec![]),
            },

            PuppetReq::NetworkConfig => {
                if let Some(nc) = runner.network_config(job_id).await {
                    RunnerResp::NetworkConfig(nc)
                } else {
                    RunnerResp::JobNotFound
                }
            }

            _ => RunnerResp::UnsupportedRequest,
        }
    }

    pub async fn new(
        job_id: Uuid,
        bind_addr: std::net::SocketAddr,
        runner: Arc<R>,
    ) -> Result<Self> {
        let server_socket: TcpListener = TcpListener::bind(bind_addr)
            .await
            .with_context(|| format!("Binding to TCP socket at {:?}", bind_addr))?;

        info!("Opened control socket TCP listener on {:?}", bind_addr);

        // let state = Arc::new(RwLock::new(ControlSocketState { client: None }));

        let (task_cmd_chan_tx, mut task_cmd_chan_rx) = tokio::sync::mpsc::channel(1);

        // let task_state = state.clone();
        let task_runner = runner.clone();

        let task_handle = tokio::spawn(async move {
            use futures::SinkExt;
            use tokio_stream::StreamExt;

            // let state = task_state;
            let runner = task_runner;

            let mut shutdown_requested = false;

            while !shutdown_requested {
                // Accept new connections. We only handle one
                // connection at any point in time:
                #[rustfmt::skip]
                let socket_res = tokio::select! {
                    accept_res = server_socket.accept() => Ok(
                        accept_res
                            .context("Accepting new control socket connection.").unwrap()
                    ),

                    cmd_res = task_cmd_chan_rx.recv() => match cmd_res {
                        Some(cmd) => Err(cmd),
                        None => {
                            panic!("Control socket command channel TX dropped!");
                        },
                    },
                };

                let socket = match socket_res {
                    Ok((socket, _client_addr)) => socket,
                    Err(ControlSocketTaskCommand::Shutdown) => {
                        shutdown_requested = true;
                        continue;
                    }
                };

                let mut transport = Framed::new(socket, LengthDelimitedCodec::new());

                loop {
                    #[rustfmt::skip]
                    let res = tokio::select! {
                        recv_res = transport.next() => {
                            match recv_res {
                                Some(Ok(bytes)) => Ok(bytes),
                                // TODO: replace with error
				// TODO: what happens on stream close?
                                Some(Err(e)) => panic!("Error occurred while receiving from control socket: {:?}", e),
				None => panic!("Error occurred while receiving from control socket: None"),
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
                    };

                    // Handle either an incoming request or a command.
                    match res {
                        Ok(bytes) => {
                            // Attept to decode the datagram. If this fails,
                            // send a RequestError response containing the error
                            // message. Otherwise, pass the request onto the
                            // handle function:
                            debug!("Trying to decode {:?}", &bytes);
                            let opt_resp = match serde_json::from_slice(&bytes) {
                                Ok(PuppetMsg::Request {
                                    request_id,
                                    request,
                                }) => Some(RunnerMsg::Response {
                                    request_id,
                                    response: Self::handle_request(
                                        request_id, request, job_id,
                                        &runner,
                                        // &mut sock_state,
                                    )
                                    .await,
                                }),
                                Ok(PuppetMsg::Event {
                                    puppet_event_id,
                                    event,
                                }) => {
                                    info!(
                                        "Received unhandled puppet event: {}, {:?}",
                                        puppet_event_id, event
                                    );
                                    None
                                }
                                Err(e) => Some(RunnerMsg::Error {
                                    message: format!("{:?}", e),
                                }),
                            };

                            if let Some(resp) = opt_resp {
                                use bytes::BufMut;

                                let mut bytes = bytes::BytesMut::new().writer();
                                serde_json::to_writer(&mut bytes, &resp)
                                    .expect("Failed to encode control socket response as JSON");

                                if let Err(e) = transport.send(bytes.into_inner().freeze()).await {
                                    match e.kind() {
                                        std::io::ErrorKind::BrokenPipe => {
                                            warn!("Puppet closed connection.");
                                            break;
                                        }
                                        _ => {
                                            warn!("Unknown error while sending answer to control socket request, ignoring: {:?}", e);
                                        }
                                    }
                                }
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

        Ok(TcpControlSocket {
            _job_id: job_id,
            task_handle,
            // state,
            task_cmd_chan: task_cmd_chan_tx,
            _runner: runner,
        })
    }

    pub async fn shutdown(self) -> Result<()> {
        log::info!("Requesting shutdown.");
        // First, request shutdown of the task:
        self.task_cmd_chan
            .send(ControlSocketTaskCommand::Shutdown)
            .await
            .with_context(|| {
                format!("Requesting shutdown of the control socket request handler")
            })?;

        // Then, try to join it:
        self.task_handle.await.unwrap().unwrap();

        // // Finally, do some cleanup.
        // let mut state = self.state.write().await;

        // TODO: figure out shutdown
        // // Joining the task implicitly drops the `UnixSeqpacketListener` to
        // // close the FD, such that we can unmount the parent file
        // // systems. However, the client connection may still be open. Shut that
        // // down:
        // state
        //     .client
        //     .take()
        //     .map(|c| c.shutdown(std::net::Shutdown::Both))
        //     .transpose()
        //     .context("Shutting down control socket TCP connection.")?;

        // The remainding cleanup happens when self is dropped.
        Ok(())
    }
}
