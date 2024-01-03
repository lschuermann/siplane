use clap::Parser;
use log::{error, info, warn};
use rendezvous_proxy::RendezvousProxy;
use std::time::Duration;
use uuid::Uuid;

/// TCP <-> WebSocket Rendezvous Proxy Application
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Client ID to send to rendezvous proxy upon listen request
    #[arg(short, long)]
    client_id: Uuid,

    /// Rendezvous server base URL
    #[arg(short = 's', long)]
    server_base_url: String,

    /// Target TCP socket address
    #[arg(short = 't', long)]
    target: String,

    /// Auth token
    #[arg(short = 'a', long)]
    auth_token: String,

    /// SSE listen channel keepalive timeout in seconds
    #[arg(long, default_value_t = 60)]
    sse_keepalive_timeout: u64,

    /// SSE reconnect wait time in seconds
    #[arg(long, default_value_t = 10)]
    sse_reconnect_wait: u64,
}

#[tokio::main]
async fn main() {
    use simplelog::{
        ColorChoice, Config as SimpleLogConfig, LevelFilter, TermLogger, TerminalMode,
    };
    use std::net::ToSocketAddrs;

    let args = Args::parse();

    // TODO: get log level from command line
    TermLogger::init(
        LevelFilter::Debug,
        SimpleLogConfig::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )
    .unwrap();

    // Get socket address from specified target:
    let sock_addr = match args.target.to_socket_addrs() {
        Ok(mut saddrs) => {
            if let Some(sa) = saddrs.next() {
                info!(
                    "Resolved target \"{}\" to socket address {:?}",
                    args.target, sa
                );
                sa
            } else {
                error!(
                    "Target \"{}\" did not resolve to any socket address",
                    args.target
                );
                return;
            }
        }

        Err(e) => {
            error!(
                "Failed to resolve \"{}\" to socket address: {:?}",
                args.target, e
            );
            return;
        }
    };

    let proxy = RendezvousProxy::start(
        args.client_id,
        args.server_base_url,
        sock_addr,
        args.auth_token,
        Duration::from_secs(args.sse_keepalive_timeout),
        Duration::from_secs(args.sse_reconnect_wait),
    )
    .await;

    info!("Proxy started, waiting for CTRL+C");
    match tokio::signal::ctrl_c().await {
        Ok(()) => {
            warn!("Received CTRL+C, shutting down!");
        }
        Err(err) => {
            error!("Unable to listen for shutdown signal: {}", err);
            // we also shut down in case of error
        }
    }

    if let Err(e) = proxy.shutdown().await {
        error!("Error while stopping proxy: {:?}", e);
    }
}
