use std::collections::HashMap;

#[macro_use]
extern crate serde;

use async_trait::async_trait;
use clap::Parser;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::Mutex;
use uuid::Uuid;

mod connector;

#[derive(Parser, Debug, Clone)]
struct NspawnRunnerArgs {
    /// Path to the TOML configuration file
    #[arg(short, long)]
    config_file: PathBuf,
}

#[derive(Deserialize, Debug, Clone)]
pub struct NspawnRunnerEnvironmentZfsRootConfig {
    // Should contain full filesystem snapshot spec
    clone_from: Option<String>,
    parent: String,
    mount_base: String,
    quota: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct NspawnRunnerEnvironmentMountConfig {
    src: String,
    dst: String,
    readonly: bool,
}

#[derive(Deserialize, Debug, Clone)]
pub struct NspawnRunnerEnvironmentConfig {
    init: String,
    shutdown_timeout: u64,
    mount: Vec<NspawnRunnerEnvironmentMountConfig>,
    zfsroot: Option<NspawnRunnerEnvironmentZfsRootConfig>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct NspawnRunnerConfig {
    coordinator_base_url: String,
    host_veth_name: String,
    board_id: Uuid,
    keepalive_timeout: u64,
    reconnect_wait: u64,
    environments: HashMap<Uuid, NspawnRunnerEnvironmentConfig>,
}

pub struct NspawnRunnerJob {
    job_id: Uuid,
    _environment_id: Uuid,
    environment_config: NspawnRunnerEnvironmentConfig,
    _ssh_keys: Vec<String>,
    nspawn_proc: tokio::process::Child,
    console_streamer_handle: tokio::task::JoinHandle<()>,
    console_streamer_cmd_chan: tokio::sync::mpsc::Sender<ConsoleStreamerCommand>,

    // Pointers to created resources, to delete when shutting down (if not
    // indicated otherwise):
    root_fs_mountpoint: Option<PathBuf>,
    zfs_root_fs: Option<String>,
}

pub struct NspawnRunner {
    connector: Arc<connector::RunnerConnector<Self>>,
    current_job: Mutex<Option<NspawnRunnerJob>>,
    config: NspawnRunnerConfig,
}

impl NspawnRunner {
    pub fn new(
        connector: Arc<connector::RunnerConnector<Self>>,
        config: NspawnRunnerConfig,
    ) -> Self {
        NspawnRunner {
            connector,
            config,
            current_job: Mutex::new(None),
        }
    }

    async fn allocate_zfs_root(
        &self,
        job_id: Uuid,
        _environment_id: Uuid,
        zfs_root_cfg: &NspawnRunnerEnvironmentZfsRootConfig,
    ) -> Result<(PathBuf, String), String> {
        let mut zfs_create_cmd = vec![
            (if zfs_root_cfg.clone_from.is_some() {
                "clone"
            } else {
                "new"
            })
            .to_string(),
            "-o".to_string(),
            "mountpoint=legacy".to_string(),
        ];

        if let Some(quota) = &zfs_root_cfg.quota {
            zfs_create_cmd.push("-o".to_string());
            zfs_create_cmd.push(format!("quota={}", quota));
        }

        if let Some(ref source_fs) = zfs_root_cfg.clone_from {
            zfs_create_cmd.push(source_fs.clone());
        }

        let zfs_fs = format!("{}/{}", zfs_root_cfg.parent, job_id.to_string());
        zfs_create_cmd.push(zfs_fs.clone());

        // Create the file system:
        match Command::new("zfs").args(&zfs_create_cmd).output().await {
            Ok(std::process::Output {
                status,
                stdout,
                stderr,
            }) => {
                if !status.success() {
                    return Err(format!(
                        "Creating root filesystem through \"zfs {:?}\" failed \
                         with exit-status {:?}. Stdout: {}, Stderr: {}",
                        &zfs_create_cmd,
                        status.code(),
                        String::from_utf8_lossy(&stdout),
                        String::from_utf8_lossy(&stderr)
                    ));
                }
            }
            Err(e) => {
                return Err(format!(
                    "Creating root filesystem through \"zfs {:?}\" failed: {:?}",
                    &zfs_create_cmd, e
                ));
            }
        }

        // Mount the file system at the indicated path. First, create this path:
        let mountpoint = format!("{}/{}", zfs_root_cfg.mount_base, job_id);
        if let Err(e) = tokio::fs::create_dir_all(&mountpoint).await {
            return Err(format!(
                "Failed to create root mountpoint {:?}: {:?}",
                &mountpoint, e,
            ));
        }

        // Now, attempt to mount it:
        match Command::new("mount")
            .args(&["-t", "zfs", &zfs_fs, &mountpoint])
            .output()
            .await
        {
            Ok(std::process::Output {
                status,
                stdout,
                stderr,
            }) => {
                if !status.success() {
                    return Err(format!(
                        "Mounting ZFS root filesystem failed with exit-status \
                         {:?}. Stdout: {}, Stderr: {}",
                        status.code(),
                        String::from_utf8_lossy(&stdout),
                        String::from_utf8_lossy(&stderr)
                    ));
                }
            }
            Err(e) => {
                return Err(format!("Mounting ZFS root filesystem failed: {:?}", e,));
            }
        }

        Ok((PathBuf::from(mountpoint), zfs_fs))
    }

    async fn destroy_zfs_root(&self, zfs_root_fs: &str) -> Result<(), String> {
        match Command::new("zfs")
            .args(&["destroy", "-v", zfs_root_fs])
            .output()
            .await
        {
            Ok(std::process::Output {
                status,
                stdout,
                stderr,
            }) => {
                if !status.success() {
                    Err(format!(
                        "Destroying ZFS root filesystem failed with exit-status \
				     {:?}. Stdout: {}, Stderr: {}",
                        status.code(),
                        String::from_utf8_lossy(&stdout),
                        String::from_utf8_lossy(&stderr)
                    ))
                } else {
                    Ok(())
                }
            }
            Err(e) => Err(format!(
                "Destroying ZFS filesystem failed with error: {:?}",
                e
            )),
        }
    }
}

enum ConsoleStreamerCommand {
    Shutdown,
}

#[async_trait]
impl connector::Runner for NspawnRunner {
    async fn start_job(
        this: &Arc<Self>,
        job_id: Uuid,
        environment_id: Uuid,
        ssh_keys: Vec<String>,
    ) {
        // This method must not block for long periods of time. We're provided
        // an &Arc<Self> to be able to launch async tasks, while returning
        // immediately. For now, we assume that all actions performed here are
        // reasonably fast, and we thus only return once the container is
        // started.

        // First, grab the `current_job` mutex. If there is already another job
        // running, we abort:
        let mut current_job_lg = this.current_job.lock().await;
        if let Some(NspawnRunnerJob {
            job_id: ref running_job_id,
            ..
        }) = *current_job_lg
        {
            this.connector
                .post_job_state(
                    job_id,
                    connector::rest_api::JobState::Failed {
                        status_message: Some(format!(
                            "Cannot start job {:?} on board {:?}, still executing job {:?}",
                            job_id, this.config.board_id, running_job_id
                        )),
                    },
                )
                .await;
            return;
        }

        // Try to get a hold of the requested job environment. Error out if the
        // environment can't be found:
        let environment_cfg = if let Some(env_cfg) = this.config.environments.get(&environment_id) {
            env_cfg
        } else {
            this.connector
                .post_job_state(
                    job_id,
                    connector::rest_api::JobState::Failed {
                        status_message: Some(format!(
                            "Cannot start job {:?} on board {:?}, unknown environment {:?}",
                            job_id, this.config.board_id, environment_id
                        )),
                    },
                )
                .await;
            return;
        };

        // We're not executing any job and acquired the lock, begin allocating
        // the root file system (volume):
        this.connector
            .post_job_state(
                job_id,
                connector::rest_api::JobState::Starting {
                    stage: connector::rest_api::JobStartingStage::Allocating,
                    status_message: None,
                },
            )
            .await;

        // Dispatch to the methods for allocating the root file system:
        let root_fs_res: Result<(PathBuf, Option<String>), String> =
            if let Some(zfs_root_cfg) = &environment_cfg.zfsroot {
                this.allocate_zfs_root(job_id, environment_id, zfs_root_cfg)
                    .await
                    .map(|(mountpoint, zfs_root_fs)| (mountpoint, Some(zfs_root_fs)))
            } else {
                Err(format!(
                    "Cannot start job {:?} on board {:?}, no root filesystem provider found.",
                    job_id, this.config.board_id,
                ))
            };

        let (root_fs_mountpoint, zfs_root_fs) = match root_fs_res {
            Ok(t) => t,
            Err(msg) => {
                this.connector
                    .post_job_state(
                        job_id,
                        connector::rest_api::JobState::Failed {
                            status_message: Some(msg),
                        },
                    )
                    .await;
                return;
            }
        };

        // All resources have been allocated, mark the container as booting:
        this.connector
            .post_job_state(
                job_id,
                connector::rest_api::JobState::Starting {
                    stage: connector::rest_api::JobStartingStage::Booting,
                    status_message: None,
                },
            )
            .await;

        let mut run_args = vec![
            "--scope".to_string(),
            "--property=DevicePolicy=closed".to_string(),
            // "--property=DeviceAllow='/dev/ttyUSB1 rw'", // To allow dev access
            "--".to_string(),
            "systemd-nspawn".to_string(),
            "-D".to_string(),
            // TODO: what to do about non-Unicode paths?
            format!("{}", root_fs_mountpoint.display()),
            "--keep-unit".to_string(),
            "--private-users=pick".to_string(),
            "--private-network".to_string(),
            format!("--network-veth-extra={}:host0", this.config.host_veth_name),
            "--bind-ro=/nix/store".to_string(),
            "--bind-ro=/nix/var/nix/db".to_string(),
            "--bind-ro=/nix/var/nix/daemon-socket".to_string(),
        ];

        // Add all additional mountpoints:
        for mount_cfg in environment_cfg.mount.iter() {
            if mount_cfg.readonly {
                run_args.push(format!("--bind-ro={}:{}", mount_cfg.src, mount_cfg.dst));
            } else {
                run_args.push(format!("--bind={}:{}", mount_cfg.src, mount_cfg.dst));
            }
        }

        // Add the init command:
        run_args.push(environment_cfg.init.to_string());

        let mut child = match tokio::process::Command::new("systemd-run")
            .args(run_args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                // TODO: cleanup root file system!
                this.connector
                    .post_job_state(
                        job_id,
                        connector::rest_api::JobState::Failed {
                            status_message: Some(format!("Failed to spawn container: {:?}", e,)),
                        },
                    )
                    .await;
                return;
            }
        };

        // Acquire the stdout and stderr pipes, and spawn a new log-streamer
        // task that collects all log output and streams it to the coordinator:
        let stdout = child
            .stdout
            .take()
            .expect("Failed to acquire stdout from child process");
        let stderr = child
            .stderr
            .take()
            .expect("Failed to acquire stderr from child process");

        let this_streamer = this.clone();
        let (streamer_chan_tx, mut streamer_chan_rx) = tokio::sync::mpsc::channel(1);
        let console_streamer = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let this = this_streamer;

            // Create BufReaders from the file descriptors for streaming:
            let mut stdout_reader = tokio::io::BufReader::with_capacity(64 * 1024, stdout);
            let mut stderr_reader = tokio::io::BufReader::with_capacity(64 * 1024, stderr);

            // We also allocate buffers (VecDeques) which are used to buffer
            // output it is acknowledged by the coordinator:
            let mut console_queue = std::collections::VecDeque::new();
            let mut console_queue_offset = 0;
            let _console_queue_sent = 0;

            let mut stdout_buf = [0; 64 * 1024];
            let mut stderr_buf = [0; 64 * 1024];

            loop {
                // TODO: force buf flush on timeout?
                let res = tokio::select!(
                        read_res = stdout_reader.read(&mut stdout_buf) => {
                            match read_res {
                    Ok(read_len) => {
                    console_queue.push_back((connector::rest_api::StdioFd::Stdout, stdout_buf[..read_len].to_vec()));
                    Ok(false)
                    }
                    Err(e) => Err(e),
                }
                        }

                        read_res = stderr_reader.read(&mut stderr_buf) => {
                match read_res {
                                Ok(read_len) => {
                    console_queue.push_back((connector::rest_api::StdioFd::Stderr, stderr_buf[..read_len].to_vec()));
                    Ok(false)
                    },
                    Err(e) => Err(e),
                            }
                        }

                streamer_cmd_opt = streamer_chan_rx.recv() => {
                match streamer_cmd_opt {
                    Some(ConsoleStreamerCommand::Shutdown) => Ok(true),
                    None => {
                    panic!("Streamer command channel TX dropped!");
                    }
                }
                }
                    );

                // TODO: wait to collect larger chunks

                match res {
                    Ok(false) => {
                        // TODO: this simply assumes that a single buffer
                        // element has been appended to the VecDeque:
                        let (stdio_fd, buf) = console_queue.back().unwrap();

                        this.connector
                            .send_job_console_log(
                                job_id,
                                console_queue_offset,
                                console_queue_offset + 1,
                                &[(*stdio_fd, buf.len())],
                                buf.clone(),
                            )
                            .await;
                        console_queue_offset += 1;
                    }

                    Ok(true) => {
                        // Asked to shut down. Once we implement chunking, do
                        // one last flush to the coordinator.
                        break;
                    }

                    Err(e) => {
                        panic!("Error reading process output: {:?}", e);
                    }
                }
            }
        });

        this.connector
            .post_job_state(
                job_id,
                connector::rest_api::JobState::Ready {
                    connection_info: vec![],
                    status_message: None,
                },
            )
            .await;

        *current_job_lg = Some(NspawnRunnerJob {
            job_id,
            _environment_id: environment_id,
            environment_config: environment_cfg.clone(),
            _ssh_keys: ssh_keys,
            nspawn_proc: child,
            console_streamer_handle: console_streamer,
            console_streamer_cmd_chan: streamer_chan_tx,
            root_fs_mountpoint: Some(root_fs_mountpoint),
            zfs_root_fs,
        });
    }

    async fn stop_job(this: &Arc<Self>, job_id: Uuid) {
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
                if job.job_id != job_id {
                    this.connector
                        .post_job_state(
                            job_id,
                            connector::rest_api::JobState::Failed {
                                status_message: Some(format!(
                                    "Cannot stop job {:?} on board {:?}, not running!",
                                    job_id, this.config.board_id,
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
                        job_id,
                        connector::rest_api::JobState::Failed {
                            status_message: Some(format!(
                                "Cannot stop job {:?} on board {:?}, not running!",
                                job_id, this.config.board_id,
                            )),
                        },
                    )
                    .await;
                return;
            }
        };

        // Take the job object, such that we own it:
        let mut job = current_job_lg.take().unwrap();

        // The requested job is currently running, procede to stop it.
        // Transition into the shutdown state:
        this.connector
            .post_job_state(
                job_id,
                connector::rest_api::JobState::Stopping {
                    status_message: None,
                },
            )
            .await;

        // First, instruct the container to shut down. We attempt a graceful
        // shutdown by sending a SIGTERM to the systemd-nspawn process, which
        // should send a SIGRTMIN+3 to the container's PID1, which will initiate
        // an orderly shutdown:
        if let Some(pid) = job.nspawn_proc.id() {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid.try_into().unwrap()),
                nix::sys::signal::Signal::SIGTERM,
            );
        }

        // Now, wait for the container to shut down, or until the shutdown
        // timeout expires:
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(job.environment_config.shutdown_timeout)) => {},
            _ = job.nspawn_proc.wait() => {}
        };

        // Attempt to get the process' exit status or, if that doesn't succeed,
        // kill it, in a loop:
        let mut exit_status = None;
        while exit_status.is_none() {
            match job.nspawn_proc.try_wait() {
                Ok(Some(es)) => {
                    exit_status = Some(es);
                }
                Ok(None) => job.nspawn_proc.kill().await.unwrap(),
                Err(e) => {
                    panic!("Error while killing nspawn process: {:?}", e);
                }
            }
        }

        // The process is dead. Instruct the log streamer to shutdown and wait
        // for the last console logs to be posted to the coordinator.
        job.console_streamer_cmd_chan
            .send(ConsoleStreamerCommand::Shutdown)
            .await
            .unwrap();
        job.console_streamer_handle.await.unwrap();

        // Unmount the container's root file system:
        if let Some(mountpoint) = job.root_fs_mountpoint {
            match Command::new("umount").args(&[mountpoint]).output().await {
                Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                }) => {
                    if !status.success() {
                        this.connector
                            .post_job_state(
                                job_id,
                                connector::rest_api::JobState::Failed {
                                    status_message: Some(format!(
                                        "Unmounting root filesystem failed with exit-status \
					 {:?}. Stdout: {}, Stderr: {}",
                                        status.code(),
                                        String::from_utf8_lossy(&stdout),
                                        String::from_utf8_lossy(&stderr)
                                    )),
                                },
                            )
                            .await;
                        return;
                    }
                }
                Err(e) => {
                    this.connector
                        .post_job_state(
                            job_id,
                            connector::rest_api::JobState::Failed {
                                status_message: Some(format!(
                                    "Unmounting root filesystem failed with error: {:?}",
                                    e
                                )),
                            },
                        )
                        .await;
                    return;
                }
            }
        }

        // If we've created a ZFS file system for this container, destroy it:
        if let Some(zfs_fs) = job.zfs_root_fs {
            if let Err(msg) = this.destroy_zfs_root(&zfs_fs).await {
                this.connector
                    .post_job_state(
                        job_id,
                        connector::rest_api::JobState::Failed {
                            status_message: Some(msg),
                        },
                    )
                    .await;
                return;
            }
        }

        // Manually drop the lock guard here, to ensure that it stays in scope
        // til the end of this function:
        core::mem::drop(current_job_lg);

        // Mark job as finished:
        this.connector
            .post_job_state(
                job_id,
                connector::rest_api::JobState::Finished {
                    status_message: None,
                },
            )
            .await;
    }
}

#[tokio::main]
async fn main() {
    let args = NspawnRunnerArgs::parse();

    let config_str = std::fs::read_to_string(args.config_file).unwrap();
    let config: NspawnRunnerConfig = toml::from_str(&config_str).unwrap();

    let mut connector_opt = None;
    let _nspawn_runner = Arc::new_cyclic(|weak_runner| {
        let connector = Arc::new(connector::RunnerConnector::new(
            config.coordinator_base_url.clone(),
            config.board_id,
            weak_runner.clone(),
            Duration::from_secs(config.keepalive_timeout),
            Duration::from_secs(config.reconnect_wait),
        ));
        connector_opt = Some(connector.clone());
        NspawnRunner::new(connector, config)
    });
    let connector = connector_opt.take().unwrap();

    connector.run().await;
}

// #[derive(Deserialize)]
// #[serde(rename_all = "snake_case")]
// #[serde(tag = "type")]
// pub enum SSEMessage {
//     UpdateState,
//     StartJob {
//         id: Uuid,
//         environment_id: String,
//         ssh_keys: Vec<String>,
//     }
// }

// #[derive(Serialize)]
// #[serde(tag = "state")]
// #[serde(rename_all = "snake_case")]
// enum JobState {
//     Starting,
//     Running,
//     Stopping,
// }

// struct NspawnRunner<'a> {
//     config: &'a Config,
//     nspawn_process: Option<tokio::process::Child>,
//     current_job: Uuid,
//     log: Vec<String>,
//     client: reqwest::Client,
// }

// impl<'a> NspawnRunner<'a> {
//     pub fn new(config: &'a Config) -> Self {
//         NspawnRunner {
//             config,
//             nspawn_process: None,
//             current_job: Uuid::nil(),
//             log: Vec::new(),
//             client: reqwest::Client::new(),
//         }
//     }

//     pub async fn start_job(&mut self, environment_id: &str, job_id: Uuid, ssh_keys: Vec<String>) {
//         println!("Starting job {:?}", job_id);
//         self.client.put(
//             &format!("{}/api/runner/v0/jobs/{}/state", self.config.coordinator_base_url, job_id.to_string())
//         )
//         .json(&JobState::Starting)
//         .send()
//         .await
//         .unwrap();

//         if self.nspawn_process.is_some() {
//             panic!("Tried to start job with existing job already running!");
//         }

//         let environment_config = self.config.environments.get(environment_id).unwrap();

//         let proc = tokio::process::Command::new("systemd-run")
//             .args([
//                 "--scope",
//                 "--property=DevicePolicy=closed",
//                 // "--property=DeviceAllow='/dev/ttyUSB1 rw'",
//                 "--",
//                 "systemd-nspawn",
//                 "-D",
//                 "/containerfs",
//                 "--keep-unit",
//                 "--private-users=pick",
//                 "--private-network",
//                 "--network-veth",
//                 "--bind-ro=/nix/store",
//                 "--bind-ro=/nix/var/nix/db",
//                 "--bind-ro=/nix/var/nix/daemon-socket",
//                 // "--bind=/dev/ttyUSB1",
//                 &environment_config.init,
//             ])
//             .spawn()
//             .expect("Failed to spawn environment");

//         self.nspawn_process = Some(proc);
//     }
// }

// async fn stream_loop<'a>(config: &Config, runner: &mut NspawnRunner<'a>) {

// }

// #[tokio::main]
// async fn main() {
//     let config_str = std::fs::read_to_string("runner_config.toml").unwrap();
//     let config: Config = toml::from_str(&config_str).unwrap();

//     let mut nspawn_runner = NspawnRunner::new(&config);

//     loop {
//         stream_loop(&config, &mut nspawn_runner).await;
//         tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

//     }
// }
