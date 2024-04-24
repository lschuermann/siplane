use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use clap::Parser;
use log::{debug, info, warn};
use serde::Deserialize;
use serial2_tokio::SerialPort;
use simplelog::{ColorChoice, Config as SimpleLogConfig, LevelFilter, TermLogger, TerminalMode};
use tokio::process::Command;
use tokio::sync::Mutex;
use uuid::Uuid;

use treadmill_rs::api::coord_runner::rest as rest_api;
use treadmill_rs::api::coord_runner::sse as sse_api;
use treadmill_rs::connector;
use treadmill_rs::control_socket;
use treadmill_rs::dummy_connector::DummyRunnerConnector;
use treadmill_sse_connector::SSERunnerConnector;
use treadmill_tcp_control_socket::TcpControlSocket;

#[derive(Parser, Debug, Clone)]
struct NetbootRunnerArgs {
    /// Path to the TOML configuration file
    #[arg(short, long)]
    config_file: PathBuf,

    /// Whether to test-start a given environment
    #[arg(long)]
    test_env: Option<Uuid>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum SSHPreferredIPVersion {
    Unspecified,
    V4,
    V6,
}

impl Default for SSHPreferredIPVersion {
    fn default() -> Self {
        SSHPreferredIPVersion::Unspecified
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct NetbootRunnerSerialConsoleConfig {
    path: std::path::PathBuf,
    baudrate: u32,
}

#[derive(Deserialize, Debug, Clone)]
pub struct NetbootRunnerEnvironmentConfig {
    // #[serde(default)]
    // init: Option<String>,
    shutdown_timeout: u64,
    // #[serde(default)]
    // mount: Vec<NetbootRunnerEnvironmentMountConfig>,
    // #[serde(default)]
    // zfsroot: Option<NetbootRunnerEnvironmentZfsRootConfig>,
    // #[serde(default)]
    // veth: Vec<NetbootRunnerEnvironmentVethConfig>,
    #[serde(default)]
    target_address_v4: Option<std::net::Ipv4Addr>,
    target_address_v6: Option<std::net::Ipv6Addr>,
    #[serde(default)]
    ssh_port: Option<u16>,
    #[serde(default)]
    ssh_preferred_ip_version: SSHPreferredIPVersion,
    #[serde(default)]
    init_script: Option<PathBuf>,
    #[serde(default)]
    reset_script: Option<PathBuf>,
    #[serde(default)]
    start_script: Option<PathBuf>,
    #[serde(default)]
    stop_script: Option<PathBuf>,

    tcp_control_socket_addr: std::net::SocketAddr,

    #[serde(default)]
    serial_console: Option<NetbootRunnerSerialConsoleConfig>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct NetbootRunnerConfig {
    coordinator_base_url: String,
    board_id: Uuid,
    keepalive_timeout: u64,
    reconnect_wait: u64,
    environments: HashMap<Uuid, NetbootRunnerEnvironmentConfig>,
}

pub struct NetbootRunnerJob {
    job_id: Uuid,
    _environment_id: Uuid,
    environment_config: NetbootRunnerEnvironmentConfig,
    ssh_keys: Vec<String>,
    console_streamer: Option<(
        tokio::task::JoinHandle<()>,
        tokio::sync::mpsc::Sender<ConsoleStreamerCommand>,
    )>,
    control_socket: TcpControlSocket<NetbootRunner>,
    ssh_rendezvous_proxies: Vec<rendezvous_proxy::RendezvousProxy>,
}

pub struct NetbootRunner {
    connector: Arc<dyn connector::RunnerConnector>,
    current_job: Mutex<Option<NetbootRunnerJob>>,
    config: NetbootRunnerConfig,
}

impl NetbootRunner {
    pub fn new(
        connector: Arc<dyn connector::RunnerConnector>,
        config: NetbootRunnerConfig,
    ) -> Self {
        NetbootRunner {
            connector,
            config,
            current_job: Mutex::new(None),
        }
    }
}

#[derive(Clone, Debug)]
enum ConsoleStreamerCommand {
    Shutdown,
}

#[async_trait]
impl connector::Runner for NetbootRunner {
    async fn start_job(this: &Arc<Self>, msg: sse_api::StartJobMessage) {
        // This method must not block for long periods of time. We're provided
        // an &Arc<Self> to be able to launch async tasks, while returning
        // immediately. For now, we assume that all actions performed here are
        // reasonably fast, and we thus only return once the container is
        // started.

        // First, grab the `current_job` mutex. If there is already another job
        // running, we abort:
        let mut current_job_lg = this.current_job.lock().await;
        if let Some(NetbootRunnerJob {
            job_id: ref running_job_id,
            ..
        }) = *current_job_lg
        {
            this.connector
                .post_job_state(
                    msg.job_id,
                    rest_api::JobState::Failed {
                        status_message: Some(format!(
                            "Cannot start job {:?} on board {:?}, still executing job {:?}",
                            msg.job_id, this.config.board_id, running_job_id
                        )),
                    },
                )
                .await;
            return;
        }

        // Try to get a hold of the requested job environment. Error out if the
        // environment can't be found:
        let environment_cfg =
            if let Some(env_cfg) = this.config.environments.get(&msg.environment_id) {
                env_cfg
            } else {
                this.connector
                    .post_job_state(
                        msg.job_id,
                        rest_api::JobState::Failed {
                            status_message: Some(format!(
                                "Cannot start job {:?} on board {:?}, unknown environment {:?}",
                                msg.job_id, this.config.board_id, msg.environment_id
                            )),
                        },
                    )
                    .await;
                return;
            };

        // TODO: prepare file systems (clone ZFS datasets, etc), mount, run
        // prepare scripts.

        // Start the control socket handler and create a new ZeroMQ socket:
        let control_socket = TcpControlSocket::new(
            msg.job_id,
            environment_cfg.tcp_control_socket_addr,
            this.clone(),
        )
        .await
        .with_context(|| {
            format!(
                "Starting TCP control socket at \"{:?}\"",
                &environment_cfg.tcp_control_socket_addr,
            )
        })
        .unwrap();

        // Spawn rendezvous proxy clients for SSH connections to the
        // container IP, if one is configured that we can reach.
        use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6};

        let ssh_socket_addr = match (
            &environment_cfg.ssh_port,
            &environment_cfg.ssh_preferred_ip_version,
            &environment_cfg.target_address_v4,
            &environment_cfg.target_address_v6,
        ) {
            (None, _, _, _) => None,
            (Some(port), SSHPreferredIPVersion::V4, Some(ipv4_addr), _) => {
                Some(SocketAddr::V4(SocketAddrV4::new(*ipv4_addr, *port)))
            }
            (Some(port), SSHPreferredIPVersion::V6, _, Some(ipv6_addr)) => {
                Some(SocketAddr::V6(SocketAddrV6::new(*ipv6_addr, *port, 0, 0)))
            }
            (Some(port), _, Some(ipv4_addr), _) => {
                Some(SocketAddr::V4(SocketAddrV4::new(*ipv4_addr, *port)))
            }
            (Some(port), _, _, Some(ipv6_addr)) => {
                Some(SocketAddr::V6(SocketAddrV6::new(*ipv6_addr, *port, 0, 0)))
            }
            _ => None,
        };

        let mut ssh_rendezvous_proxies = Vec::with_capacity(msg.ssh_rendezvous_servers.len());
        if let Some(sa) = ssh_socket_addr {
            for server_spec in &msg.ssh_rendezvous_servers {
                ssh_rendezvous_proxies.push(
                    rendezvous_proxy::RendezvousProxy::start(
                        server_spec.client_id.clone(),
                        server_spec.server_base_url.clone(),
                        sa,
                        server_spec.auth_token.clone(),
                        Duration::from_secs(60),
                        Duration::from_secs(10),
                    )
                    .await,
                );
            }
        }

        // Run init script, if we have one:
        if let Some(init_script) = &environment_cfg.init_script {
            info!("Running init_script {:?}...", init_script);
            let out = Command::new(init_script)
                .env("TML_JOB_ID", msg.job_id.to_string())
                .output()
                .await
                .expect("Failed running init_script command");
            info!("Ran init_script: {:?}", out);
        } else {
            warn!("No init_script provided, skipping!");
        }

        // Run start script, if we have one:
        if let Some(start_script) = &environment_cfg.start_script {
            info!("Running start_script {:?}...", start_script);
            let out = Command::new(start_script)
                .env("TML_JOB_ID", msg.job_id.to_string())
                .output()
                .await
                .expect("Failed running start_script command");
            info!("Ran start_script: {:?}", out);
        } else {
            warn!("No start_script provided, skipping!");
        }

        // All resources have been allocated, mark the target as booting:
        this.connector
            .post_job_state(
                msg.job_id,
                rest_api::JobState::Starting {
                    stage: rest_api::JobStartingStage::Booting,
                    status_message: None,
                },
            )
            .await;

        // Connect to the serial port:
        let console_streamer_handles = if let Some(console_serial_port) = environment_cfg
            .serial_console
            .as_ref()
            .and_then(|serial_console_cfg| {
                match SerialPort::open(
                    serial_console_cfg.path.to_str().unwrap(),
                    serial_console_cfg.baudrate,
                ) {
                    Ok(serialport) => Some(serialport),
                    Err(e) => {
                        warn!("Unable to open serial port: {:?}", e);
                        None
                    }
                }
            }) {
            let this_streamer = this.clone();
            let (streamer_chan_tx, mut streamer_chan_rx) = tokio::sync::mpsc::channel(1);
            let console_streamer = tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let this = this_streamer;

                // Create BufReaders from the file descriptors for streaming:
                let mut buffered_reader =
                    tokio::io::BufReader::with_capacity(64 * 1024, console_serial_port);

                // We also allocate buffers (VecDeques) which are used to buffer
                // output it is acknowledged by the coordinator:
                let mut console_queue = std::collections::VecDeque::<Vec<u8>>::new();
                let mut console_queue_offset = 0;
                let _console_queue_sent = 0;

                let mut read_buf = [0; 64 * 1024];
                let mut reader_closed = false;

                enum ReadConsoleRes {
                    ZeroBytes,
                    Data,
                    Shutdown,
                    Error(std::io::Error),
                }

                loop {
                    // TODO: force buf flush on timeout?
                    #[rustfmt::skip]
                    let res = tokio::select! {
			streamer_cmd_opt = streamer_chan_rx.recv() => {
                            match streamer_cmd_opt {
				Some(ConsoleStreamerCommand::Shutdown) => ReadConsoleRes::Shutdown,
				None => {
                                    panic!("Streamer command channel TX dropped!");
				}
                            }
			}

			read_res = buffered_reader.read(&mut read_buf), if !reader_closed => {
			    match read_res {
			        Ok(0) => {
			            // Mark as closed, so we don't loop reading zero bytes:
			            reader_closed = true;
			            ReadConsoleRes::ZeroBytes
			        },
			        Ok(read_len) => {
			            console_queue.push_back(
			                read_buf[..read_len].to_vec()
			            );
			            ReadConsoleRes::Data
			        }
			        Err(e) => ReadConsoleRes::Error(e),
			    }
			}

			// read_res = stderr_reader.read(&mut stderr_buf), if !stderr_closed => {
			//     match read_res {
			//         Ok(0) => {
			//             // Mark as closed, so we don't loop reading zero bytes:
			//             stderr_closed = true;
			//             ReadConsoleRes::ZeroBytes
			//         },
			//         Ok(read_len) => {
			//             console_queue.push_back((
			//                 rest_api::StdioFd::Stderr,
			//                 stderr_buf[..read_len].to_vec()
			//             ));
			//             ReadConsoleRes::Data
			//         },
			//         Err(e) => ReadConsoleRes::Error(e),
			//     }
			// }
                    };

                    match res {
                        ReadConsoleRes::Data => {
                            // TODO: this simply assumes that a single buffer
                            // element has been appended to the VecDeque:
                            let buf = console_queue.back().unwrap();

                            this.connector
                                .send_job_console_log(
                                    msg.job_id,
                                    console_queue_offset,
                                    console_queue_offset + 1,
                                    &[(rest_api::StdioFd::Stdout, buf.len())],
                                    buf.clone(),
                                )
                                .await;
                            console_queue_offset += 1;
                        }

                        ReadConsoleRes::Shutdown => {
                            // Asked to shut down. Once we implement chunking, do
                            // one last flush to the coordinator.
                            debug!("Shutting down console log streamer.");
                            break;
                        }

                        ReadConsoleRes::Error(e) => {
                            panic!("Error reading from serial port: {:?}", e);
                        }

                        ReadConsoleRes::ZeroBytes => {
                            // TODO: still need this case?
                        }
                    }
                }
            });

            Some((console_streamer, streamer_chan_tx))
        } else {
            None
        };

        // TODO: it'd be nice if this didn't have to be
        // sequential. But using tokio's JoinSet we get lifetime
        // issues here, as .spawn() requires a 'static borrow of the
        // rendezvous proxies.
        let mut rendezvous_proxy_addrs = vec![];
        for proxy in ssh_rendezvous_proxies.iter() {
            match proxy.public_addr(Duration::from_secs(5)).await {
                Some((hostname, port)) => {
                    rendezvous_proxy_addrs.push(
                        rest_api::JobSessionConnectionInfo::RendezvousSSH {
                            hostname,
                            port,
                            host_key_fingerprints: vec![],
                        },
                    );
                }
                None => {
                    warn!("Rendezvous proxy did not provide public address before timeout.");
                }
            }
        }

        this.connector
            .post_job_state(
                msg.job_id,
                rest_api::JobState::Ready {
                    connection_info: rendezvous_proxy_addrs,
                    status_message: None,
                },
            )
            .await;

        *current_job_lg = Some(NetbootRunnerJob {
            job_id: msg.job_id,
            _environment_id: msg.environment_id,
            environment_config: environment_cfg.clone(),
            ssh_keys: msg.ssh_keys,
            console_streamer: console_streamer_handles,
            // root_fs_mountpoint: Some(root_fs_mountpoint),
            ssh_rendezvous_proxies,
            control_socket,
        });
    }

    async fn stop_job(this: &Arc<Self>, msg: sse_api::StopJobMessage) {
        // This method must not block for long periods of time. We're provided
        // an &Arc<Self> to be able to launch async tasks, while returning
        // immediately. For now, we assume that all actions performed here are
        // reasonably fast, and we thus only return once the container is
        // started.

        // First, grab the `current_job` mutex. If there is already another job
        // running, we abort. We remove take the job object from the option, but
        // to prevent another task to race with this method, hold the lock guard
        // til the very end:
        let mut current_job_lg = this.current_job.lock().await;
        match *current_job_lg {
            Some(ref job) => {
                if job.job_id != msg.job_id {
                    this.connector
                        .post_job_state(
                            msg.job_id,
                            rest_api::JobState::Failed {
                                status_message: Some(format!(
                                    "Cannot stop job {:?} on board {:?}, not running!",
                                    msg.job_id, this.config.board_id,
                                )),
                            },
                        )
                        .await;
                    return;
                }
            }

            None => {
                this.connector
                    .post_job_state(
                        msg.job_id,
                        rest_api::JobState::Failed {
                            status_message: Some(format!(
                                "Cannot stop job {:?} on board {:?}, not running!",
                                msg.job_id, this.config.board_id,
                            )),
                        },
                    )
                    .await;
                return;
            }
        };

        // Take the job object, such that we own it:
        let job = current_job_lg.take().unwrap();

        // The requested job is currently running, procede to stop it.
        // Transition into the shutdown state:
        this.connector
            .post_job_state(
                msg.job_id,
                rest_api::JobState::Stopping {
                    status_message: None,
                },
            )
            .await;

        // TODO: request orderly shutdown on the control socket

        // TODO: Run the stop script

        // The child is dead. Destroy the control socket:
        job.control_socket.shutdown().await.unwrap();

        // Instruct the log streamer to shutdown and wait for the last console
        // logs to be posted to the coordinator.
        if let Some((task_handle, cmd_chan)) = job.console_streamer {
            debug!("Requesting console streamer to shut down.");
            cmd_chan
                .send(ConsoleStreamerCommand::Shutdown)
                .await
                .expect("Console streamer task has quit before receiving shutdown signal!");
            task_handle.await.unwrap();
            debug!("Console streamer has shut down.");

            // Shut down all rendezvous proxy clients:
            for proxy in job.ssh_rendezvous_proxies {
                if let Err(e) = proxy.shutdown().await {
                    warn!("Error while shutting down rendezvous proxy client: {:?}", e);
                }
            }
        }

        // Manually drop the lock guard here, to ensure that it stays in scope
        // til the end of this function:
        core::mem::drop(current_job_lg);

        // Mark job as finished:
        this.connector
            .post_job_state(
                msg.job_id,
                rest_api::JobState::Finished {
                    status_message: None,
                },
            )
            .await;
    }
}

#[async_trait]
impl control_socket::Runner for NetbootRunner {
    async fn ssh_keys(&self, tgt_job_id: Uuid) -> Option<Vec<String>> {
        match *self.current_job.lock().await {
            None => None,
            Some(NetbootRunnerJob {
                ref job_id,
                ref ssh_keys,
                ..
            }) if *job_id == tgt_job_id => Some(ssh_keys.clone()),
            Some(_) => None,
        }
    }

    async fn network_config(
        &self,
        tgt_job_id: Uuid,
    ) -> Option<treadmill_rs::api::runner_puppet::NetworkConfig> {
        match *self.current_job.lock().await {
            None => None,
            Some(NetbootRunnerJob { ref job_id, .. }) if *job_id == tgt_job_id => {
                let hostname = format!("job-{}", format!("{}", job_id).split_at(10).0);
                Some(treadmill_rs::api::runner_puppet::NetworkConfig {
                    hostname,
                    interface: None,
                    ipv4: None,
                    ipv6: None,
                })
            }
            Some(_) => None,
        }
    }
}

#[tokio::main]
async fn main() {
    use treadmill_rs::connector::RunnerConnector;

    TermLogger::init(
        LevelFilter::Debug,
        SimpleLogConfig::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )
    .unwrap();
    info!("Starting treadmill netboot runner.");

    let args = NetbootRunnerArgs::parse();

    let config_str = std::fs::read_to_string(args.config_file).unwrap();
    let config: NetbootRunnerConfig = toml::from_str(&config_str).unwrap();

    if let Some(test_env_uuid) = args.test_env {
        let mut connector_opt = None;
        let _netboot_runner = Arc::new_cyclic(|weak_runner| {
            let connector = Arc::new(DummyRunnerConnector::new(
                config.board_id,
                test_env_uuid,
                weak_runner.clone(),
            ));
            connector_opt = Some(connector.clone());
            NetbootRunner::new(connector, config)
        });
        let connector = connector_opt.take().unwrap();
        connector.run().await;
    } else {
        let mut connector_opt = None;
        let _netboot_runner = Arc::new_cyclic(|weak_runner| {
            let connector = Arc::new(SSERunnerConnector::new(
                config.coordinator_base_url.clone(),
                config.board_id,
                weak_runner.clone(),
                Duration::from_secs(config.keepalive_timeout),
                Duration::from_secs(config.reconnect_wait),
            ));
            connector_opt = Some(connector.clone());
            NetbootRunner::new(connector, config)
        });
        let connector = connector_opt.take().unwrap();
        connector.run().await;
    }
}
