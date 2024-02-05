use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, Result};
use bytes::{BufMut, Bytes, BytesMut};
use clap::{Parser, ValueEnum};
use log::{debug, error, info, warn};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_seqpacket::UnixSeqpacket;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use treadmill_rs::api::runner_puppet::{
    NetworkConfig, PuppetEvent, PuppetMsg, PuppetReq, RunnerMsg, RunnerResp,
};

enum UnixSeqpacketControlSocketClientTaskCmd {
    Shutdown,
}

struct UnixSeqpacketControlSocketClient {
    socket: Arc<UnixSeqpacket>,
    puppet_event_cnt: Mutex<u64>,
    request_responses: Arc<Mutex<(u64, HashMap<u64, Option<RunnerResp>>)>>,
    task_cmd_tx: tokio::sync::mpsc::Sender<UnixSeqpacketControlSocketClientTaskCmd>,
    task_notify: Arc<tokio::sync::Notify>,
    task_join_handle: tokio::task::JoinHandle<()>,
}

impl UnixSeqpacketControlSocketClient {
    async fn new<P: AsRef<Path>>(
        unix_seqpacket_control_socket: P,
    ) -> Result<UnixSeqpacketControlSocketClient> {
        let socket = Arc::new(
            UnixSeqpacket::connect(&unix_seqpacket_control_socket)
                .await
                .with_context(|| {
                    format!(
                        "Opening UNIX SeqPacket control socket connection at {:?}",
                        unix_seqpacket_control_socket.as_ref(),
                    )
                })?,
        );

        let request_responses = Arc::new(Mutex::new((0, HashMap::new())));

        let task_socket = socket.clone();
        let task_request_responses = request_responses.clone();
        let task_notify = Arc::new(tokio::sync::Notify::new());
        let task_notify_task = task_notify.clone();
        let (task_cmd_tx, task_cmd_rx) = tokio::sync::mpsc::channel(1);

        let task_join_handle = tokio::spawn(async move {
            Self::task(
                task_socket,
                task_request_responses,
                task_cmd_rx,
                task_notify_task,
            )
            .await
        });

        Ok(UnixSeqpacketControlSocketClient {
            socket,
            puppet_event_cnt: Mutex::new(0),
            request_responses,
            task_cmd_tx,
            task_notify,
            task_join_handle,
        })
    }

    pub async fn shutdown(self) {
        info!("Requesting runner socket client to shut down...");
        self.task_cmd_tx
            .send(UnixSeqpacketControlSocketClientTaskCmd::Shutdown)
            .await
            .expect("Runner socket client task has quit before receiving shutdown signal!");
        self.task_join_handle.await.unwrap();
    }

    async fn task(
        socket: Arc<UnixSeqpacket>,
        request_responses: Arc<Mutex<(u64, HashMap<u64, Option<RunnerResp>>)>>,
        mut cmd_rx: tokio::sync::mpsc::Receiver<UnixSeqpacketControlSocketClientTaskCmd>,
        notify: Arc<tokio::sync::Notify>,
    ) {
        let mut recv_buf = vec![0; 1024 * 1024];

        loop {
            tokio::select! {
            cmd_res = cmd_rx.recv() => {
                match cmd_res {
                None => {
                    panic!("Task command channel TX dropped before shutdown!");
                },

                Some(UnixSeqpacketControlSocketClientTaskCmd::Shutdown) => {
                    debug!("Shutting down runner socket client");
                    break;
                },
                }
            }

            size_res = socket.recv(&mut recv_buf) => {
                let size = match size_res {
                Err(e) => {
                    error!("Failed to receive runner message: {:?}", e);
                    continue;
                }
                Ok(s) => s,
                };

                match serde_json::from_slice(&recv_buf[..size]) {
                Ok(RunnerMsg::Response {
                    request_id,
                    response,
                }) => {
                    let resp_map = &mut request_responses.lock().await.1;
                    if let Some(entry) = resp_map.get_mut(&request_id) {
                    if entry.is_some() {
                        error!("Received spurious response for request ID {}: {:?}",
                           request_id, response);
                    }
                    *entry = Some(response);
                    notify.notify_waiters();
                    } else {
                    error!("Received response for unexpected request ID {}: {:?}",
                           request_id, response);
                    }
                },

                Ok(RunnerMsg::Event {
                    runner_event_id,
                    event,
                }) => {
                    warn!("Received unhandled runner event with id {}: {:?}",
                      runner_event_id, event);
                }

                Ok(RunnerMsg::Error {
                    message,
                }) => {
                    warn!("Received error message from runner: {:?}", message);
                }

                Err(e) => {
                    panic!("Couldn't parse runner message: {:?}", e);
                }
                }
            }
            }
        }
    }

    async fn request(&self, req: PuppetReq) -> RunnerResp {
        let request_id = {
            // Acquire request ID:
            let mut request_responses_lg = self.request_responses.lock().await;

            let request_id = request_responses_lg.0;
            request_responses_lg.0 = request_responses_lg
                .0
                .checked_add(1)
                .expect("Request counter overflow!");

            // Insert dummy value, to indicate that we're actually waiting on this
            // request. This helps debug cases where the runner sends a response to
            // an invalid request ID or a request that is no longer current:
            assert!(request_responses_lg.1.insert(request_id, None).is_none());

            request_id
        };

        // While we're not holding the lock, send the request:
        self.socket
            .send(
                &serde_json::to_vec(&PuppetMsg::Request {
                    request_id,
                    request: req,
                })
                .unwrap(),
            )
            .await
            .unwrap();

        // Re-acquire the lock:
        let mut request_responses_lg = self.request_responses.lock().await;

        // Now, while we're hold the lock guard, request a notification, but
        // only await it after releasing the lock to avoid a deadlock:
        while request_responses_lg.1.get(&request_id).unwrap().is_none() {
            let fut = self.task_notify.notified();
            std::mem::drop(request_responses_lg);
            fut.await;
            request_responses_lg = self.request_responses.lock().await;
        }

        // We have a response, extract and return it:
        request_responses_lg.1.remove(&request_id).unwrap().unwrap()
    }

    async fn send_event(&self, ev: PuppetEvent) {
        let event_id = {
            let mut puppet_event_cnt = self.puppet_event_cnt.lock().await;
            let event_id = *puppet_event_cnt;
            *puppet_event_cnt = puppet_event_cnt
                .checked_add(1)
                .expect("Puppet event ID overflow!");
            event_id
        };

        self.socket
            .send(
                &serde_json::to_vec(&PuppetMsg::Event {
                    puppet_event_id: event_id,
                    event: ev,
                })
                .unwrap(),
            )
            .await
            .unwrap();
    }
}

enum TcpControlSocketClientTaskCmd {
    SendMessage(Bytes),
    Shutdown,
}

struct TcpControlSocketClient {
    puppet_event_cnt: Mutex<u64>,
    request_responses: Arc<Mutex<(u64, HashMap<u64, Option<RunnerResp>>)>>,
    task_cmd_tx: tokio::sync::mpsc::Sender<TcpControlSocketClientTaskCmd>,
    task_notify: Arc<tokio::sync::Notify>,
    task_join_handle: tokio::task::JoinHandle<()>,
}

impl TcpControlSocketClient {
    async fn new(addr: std::net::SocketAddr) -> Result<TcpControlSocketClient> {
        let socket = TcpStream::connect(addr)
            .await
            .with_context(|| format!("Opening TCP control socket connection at {:?}", addr,))?;

        let request_responses = Arc::new(Mutex::new((0, HashMap::new())));

        let task_request_responses = request_responses.clone();
        let task_notify = Arc::new(tokio::sync::Notify::new());
        let task_notify_task = task_notify.clone();
        let (task_cmd_tx, task_cmd_rx) = tokio::sync::mpsc::channel(1);

        let task_join_handle = tokio::spawn(async move {
            Self::task(
                socket,
                task_request_responses,
                task_cmd_rx,
                task_notify_task,
            )
            .await
        });

        Ok(TcpControlSocketClient {
            puppet_event_cnt: Mutex::new(0),
            request_responses,
            task_cmd_tx,
            task_notify,
            task_join_handle,
        })
    }

    pub async fn shutdown(self) {
        info!("Requesting runner socket client to shut down...");
        self.task_cmd_tx
            .send(TcpControlSocketClientTaskCmd::Shutdown)
            .await
            .expect("Runner socket client task has quit before receiving shutdown signal!");
        self.task_join_handle.await.unwrap();
    }

    async fn task(
        socket: TcpStream,
        request_responses: Arc<Mutex<(u64, HashMap<u64, Option<RunnerResp>>)>>,
        mut cmd_rx: tokio::sync::mpsc::Receiver<TcpControlSocketClientTaskCmd>,
        notify: Arc<tokio::sync::Notify>,
    ) {
        use futures::SinkExt;
        use tokio_stream::StreamExt;

        let mut transport = Framed::new(socket, LengthDelimitedCodec::new());

        loop {
            #[rustfmt::skip]
            tokio::select! {
		cmd_res = cmd_rx.recv() => {
                    match cmd_res {
			None => {
			    panic!("Task command channel TX dropped before shutdown!");
			},

			Some(TcpControlSocketClientTaskCmd::Shutdown) => {
			    debug!("Shutting down runner socket client");
			    break;
			},

			Some(TcpControlSocketClientTaskCmd::SendMessage(bytes)) => {
			    match transport.send(bytes).await {
				Ok(()) => (),
				Err(e) => {
				    error!("Error sending message to runner: {:?}", e);
				}
			    }
			},
                    }
		}

		recv_res = transport.next() => {
                    let bytes = match recv_res {
			Some(Err(e)) => {
			    // maybe this contains end of stream?
			    error!("Failed to receive runner message: {:?}", e);
			    continue;
			}
			Some(Ok(b)) => b,
			None => {
			    // TODO: this is likely end of stream?
			    error!("Failed to receive runner message: None");
			    continue;
			}
                    };

                    match serde_json::from_slice(&bytes) {
			Ok(RunnerMsg::Response {
			    request_id,
			    response,
			}) => {
			    let resp_map = &mut request_responses.lock().await.1;
			    if let Some(entry) = resp_map.get_mut(&request_id) {
				if entry.is_some() {
				    error!("Received spurious response for request ID {}: {:?}",
					   request_id, response);
				}
				*entry = Some(response);
				notify.notify_waiters();
			    } else {
				error!("Received response for unexpected request ID {}: {:?}",
				       request_id, response);
			    }
			},

			Ok(RunnerMsg::Event {
			    runner_event_id,
			    event,
			}) => {
			    warn!("Received unhandled runner event with id {}: {:?}",
				  runner_event_id, event);
			}

			Ok(RunnerMsg::Error {
			    message,
			}) => {
			    warn!("Received error message from runner: {:?}", message);
			}

			Err(e) => {
			    panic!("Couldn't parse runner message: {:?}", e);
			}
                    }
		}
            }
        }
    }

    async fn request(&self, req: PuppetReq) -> RunnerResp {
        let request_id = {
            // Acquire request ID:
            let mut request_responses_lg = self.request_responses.lock().await;

            let request_id = request_responses_lg.0;
            request_responses_lg.0 = request_responses_lg
                .0
                .checked_add(1)
                .expect("Request counter overflow!");

            // Insert dummy value, to indicate that we're actually waiting on this
            // request. This helps debug cases where the runner sends a response to
            // an invalid request ID or a request that is no longer current:
            assert!(request_responses_lg.1.insert(request_id, None).is_none());

            request_id
        };

        // Ask the async task to send the request:
        let mut bytes = BytesMut::new().writer();
        serde_json::to_writer(
            &mut bytes,
            &PuppetMsg::Request {
                request_id,
                request: req,
            },
        )
        .expect("Failed to encode control socket request as JSON");
        self.task_cmd_tx
            .send(TcpControlSocketClientTaskCmd::SendMessage(
                bytes.into_inner().freeze(),
            ))
            .await
            .expect("Runner socket client task is no longer alive!");

        // Re-acquire the lock:
        let mut request_responses_lg = self.request_responses.lock().await;

        // Now, while we're hold the lock guard, request a notification, but
        // only await it after releasing the lock to avoid a deadlock:
        while request_responses_lg.1.get(&request_id).unwrap().is_none() {
            let fut = self.task_notify.notified();
            std::mem::drop(request_responses_lg);
            fut.await;
            request_responses_lg = self.request_responses.lock().await;
        }

        // We have a response, extract and return it:
        request_responses_lg.1.remove(&request_id).unwrap().unwrap()
    }

    async fn send_event(&self, ev: PuppetEvent) {
        let event_id = {
            let mut puppet_event_cnt = self.puppet_event_cnt.lock().await;
            let event_id = *puppet_event_cnt;
            *puppet_event_cnt = puppet_event_cnt
                .checked_add(1)
                .expect("Puppet event ID overflow!");
            event_id
        };

        // Ask the async task to send the event:
        let mut bytes = BytesMut::new().writer();
        serde_json::to_writer(
            &mut bytes,
            &PuppetMsg::Event {
                puppet_event_id: event_id,
                event: ev,
            },
        )
        .expect("Failed to encode control socket event as JSON");
        self.task_cmd_tx
            .send(TcpControlSocketClientTaskCmd::SendMessage(
                bytes.into_inner().freeze(),
            ))
            .await
            .expect("Runner socket client task is no longer alive!");
    }
}

enum ControlSocketClient {
    UnixSeqpacket(UnixSeqpacketControlSocketClient),
    Tcp(TcpControlSocketClient),
}

impl ControlSocketClient {
    async fn request(&self, req: PuppetReq) -> RunnerResp {
        match self {
            ControlSocketClient::UnixSeqpacket(client) => client.request(req).await,
            ControlSocketClient::Tcp(client) => client.request(req).await,
        }
    }

    async fn send_event(&self, ev: PuppetEvent) {
        match self {
            ControlSocketClient::UnixSeqpacket(client) => client.send_event(ev).await,
            ControlSocketClient::Tcp(client) => client.send_event(ev).await,
        }
    }

    pub async fn shutdown(self) {
        match self {
            ControlSocketClient::UnixSeqpacket(client) => client.shutdown().await,
            ControlSocketClient::Tcp(client) => client.shutdown().await,
        }
    }

    pub async fn get_ssh_keys(&self) -> Vec<String> {
        let resp = self.request(PuppetReq::SSHKeys).await;
        match resp {
            RunnerResp::SSHKeysResp { ssh_keys } => ssh_keys,
            _ => {
                panic!("Invalid runner response to SSH keys request: {:?}", resp);
            }
        }
    }

    pub async fn get_network_config(&self) -> NetworkConfig {
        let resp = self.request(PuppetReq::NetworkConfig).await;
        match resp {
            RunnerResp::NetworkConfig(nc) => nc,
            _ => {
                panic!(
                    "Invalid runner response to network config request: {:?}",
                    resp
                );
            }
        }
    }

    pub async fn report_ready(&self) {
        self.send_event(PuppetEvent::Ready).await
    }
}

#[derive(Debug, Clone, ValueEnum)]
#[clap(rename_all = "snake_case")]
enum PuppetControlSocketTransport {
    UnixSeqpacket,
    Tcp,
}

#[derive(Debug, Clone, Parser)]
struct PuppetArgs {
    #[arg(long, short = 't')]
    transport: PuppetControlSocketTransport,

    #[arg(long, required_if_eq("transport", "unix_seqpacket"))]
    unix_seqpacket_control_socket: Option<PathBuf>,

    #[arg(long, required_if_eq("transport", "tcp"))]
    tcp_control_socket_addr: Option<std::net::SocketAddr>,

    #[arg(long)]
    authorized_keys_file: Option<PathBuf>,

    #[arg(long)]
    network_config_script: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    use simplelog::{
        ColorChoice, Config as SimpleLogConfig, LevelFilter, TermLogger, TerminalMode,
    };
    TermLogger::init(
        LevelFilter::Debug,
        SimpleLogConfig::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )
    .unwrap();

    let args = PuppetArgs::parse();

    let client = match args.transport {
        PuppetControlSocketTransport::UnixSeqpacket => ControlSocketClient::UnixSeqpacket(
            UnixSeqpacketControlSocketClient::new(&args.unix_seqpacket_control_socket.unwrap())
                .await?,
        ),

        PuppetControlSocketTransport::Tcp => ControlSocketClient::Tcp(
            TcpControlSocketClient::new(args.tcp_control_socket_addr.unwrap()).await?,
        ),
    };

    if let Some(ref authorized_keys_file) = args.authorized_keys_file {
        // Request the SSH keys:
        let ssh_keys = client.get_ssh_keys().await;

        // Create the authorized keys file's parent directories (if they
        // don't exist) and dump the keys to the file:
        tokio::fs::create_dir_all(authorized_keys_file.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(authorized_keys_file, ssh_keys.join("\n").as_bytes())
            .await
            .unwrap();
    }

    // Request the network configuration, dump it into environment variables and
    // pass it onto the network configuration script, if one is provided:
    if let Some(script) = &args.network_config_script {
        let network_config = client.get_network_config().await;

        let mut cmd = tokio::process::Command::new(script);
        cmd.stdin(Stdio::null());
        cmd.env("HOSTNAME", &network_config.hostname);

        if let Some(ref iface) = network_config.interface {
            cmd.env("INTERFACE", iface);
        }

        if let Some(ref v4_config) = network_config.ipv4 {
            cmd.env("IPV4_ADDRESS", format!("{}", v4_config.address));
            cmd.env("IPV4_PREFIX_LENGTH", format!("{}", v4_config.prefix_length));
            if let Some(ref v4_gw) = v4_config.gateway {
                cmd.env("IPV4_GATEWAY", format!("{}", v4_gw));
            }
            let nameserver_str: String = v4_config
                .nameservers
                .iter()
                .map(|addr| format!("{}", addr))
                // This is much cleaner with the nightly-only .intersperse
                .fold(String::new(), |acc, nameserver| {
                    let sep = if acc.len() != 0 { "|" } else { "" };
                    acc + sep + &nameserver
                });
            cmd.env("IPV4_NAMESERVERS", nameserver_str);
        }

        if let Some(ref v6_config) = network_config.ipv6 {
            cmd.env("IPV6_ADDRESS", format!("{}", v6_config.address));
            cmd.env("IPV6_PREFIX_LENGTH", format!("{}", v6_config.prefix_length));
            if let Some(ref v6_gw) = v6_config.gateway {
                cmd.env("IPV6_GATEWAY", format!("{}", v6_gw));
            }
            let nameserver_str: String = v6_config
                .nameservers
                .iter()
                .map(|addr| format!("{}", addr))
                // This is much cleaner with the nightly-only .intersperse
                .fold(String::new(), |acc, nameserver| {
                    let sep = if acc.len() != 0 { "|" } else { "" };
                    acc + sep + &nameserver
                });
            cmd.env("IPV6_NAMESERVERS", nameserver_str);
        }

        match cmd.spawn() {
            Ok(mut child) => {
                match child.wait().await {
                    Ok(status) => {
                        if let Some(code) = status.code() {
                            if code == 0 {
                                info!("Successfully configured networking.");
                            } else {
                                warn!("Network configuration script reported non-zero exit status: {}", code);
                            }
                        } else {
                            warn!("Network configuration script terminated by a signal.");
                        }
                    }
                    Err(e) => {
                        error!("Error running network configuration script: {:?}", e);
                    }
                }
            }

            Err(e) => {
                error!("Error spawning network configuration script: {:?}", e);
            }
        }
    }

    // Report the puppet as ready:
    client.report_ready().await;

    info!("Puppet started, waiting for CTRL+C");
    match tokio::signal::ctrl_c().await {
        Ok(()) => {
            warn!("Received CTRL+C, shutting down!");
        }
        Err(err) => {
            error!("Unable to listen for shutdown signal: {}", err);
            // we also shut down in case of error
        }
    }

    client.shutdown().await;

    Ok(())
}
