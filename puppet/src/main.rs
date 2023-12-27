use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use tokio_seqpacket::UnixSeqpacket;

use siplane_rs::api::runner_puppet::{PuppetEvent, PuppetMsg, PuppetReq, RunnerMsg, RunnerResp};

#[derive(Debug, Clone, Parser)]
struct PuppetArgs {
    #[arg(long, short = 's')]
    unix_seqpacket_control_socket: PathBuf,

    #[arg(long)]
    authorized_keys_file: PathBuf,
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

    let socket = UnixSeqpacket::connect(&args.unix_seqpacket_control_socket)
        .await
        .with_context(|| {
            format!(
                "Opening UNIX SeqPacket control socket connection at {:?}",
                args.unix_seqpacket_control_socket
            )
        })?;

    // Request the SSH keys:
    socket
        .send(
            &serde_json::to_vec(&PuppetMsg::Request {
                request_id: 0,
                request: PuppetReq::SSHKeys,
            })
            .unwrap(),
        )
        .await
        .unwrap();

    // Expect the first response to be the list of SSH keys:
    let mut recv_buf = vec![0; 1024 * 1024];
    let size = socket.recv(&mut recv_buf).await.unwrap();
    let ssh_keys = match serde_json::from_slice(&recv_buf[..size]) {
        Ok(RunnerMsg::Response {
            request_id,
            response,
        }) if request_id == 0 => match response {
            RunnerResp::SSHKeysResp { ssh_keys } => ssh_keys,
            _ => panic!("Invalid response discriminator: {:?}", response),
        },
        Ok(resp) => {
            panic!("Received unexpected response: {:?}", resp);
        }
        Err(e) => {
            panic!("Couldn't parse response: {:?}", e);
        }
    };

    // Create the authorized keys file's parent directories (if they
    // don't exist) and dump the keys to the file:
    tokio::fs::create_dir_all(args.authorized_keys_file.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&args.authorized_keys_file, ssh_keys.join("\n").as_bytes())
        .await
        .unwrap();

    // Report the puppet as ready:
    socket
        .send(
            &serde_json::to_vec(&PuppetMsg::Event {
                puppet_event_id: 0,
                event: PuppetEvent::Ready,
            })
            .unwrap(),
        )
        .await
        .unwrap();

    // For now, we just wait forever. At some point we should
    // implement a proper event handler.
    std::future::pending::<()>().await;

    Ok(())
}
