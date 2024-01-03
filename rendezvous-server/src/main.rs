#[macro_use]
extern crate rocket;

use std::collections::{HashMap, LinkedList};
use std::net::{IpAddr, SocketAddr};
use std::ops::Range;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use rocket::response::stream::{Event, EventStream};
use rocket::State;
use rocket_ws as ws;
use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tokio::sync::Mutex;
use uuid::Uuid;

// enum ListenerStreamCommand {
//     NewConnectionEvent(listener_sse_api::NewConnection),
//     Close,
// }

// TODO: share between server and proxy
mod listener_sse_api {
    use serde::Serialize;

    #[derive(Debug, Clone, Serialize)]
    pub struct Listening {
        pub public_hostname: String,
        pub public_port: u16,
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct NewConnection {
        pub client_ip: String,
        pub client_port: u16,
        pub remote_ip: String,
        pub remote_port: u16,
        pub rendezvous_endpoint: String,
        pub accept_token: String,
    }

    #[derive(Debug, Clone, Serialize)]
    #[serde(rename_all = "snake_case")]
    #[serde(tag = "type")]
    pub enum SSEMessage {
        Listening(Listening),
        NewConnection(NewConnection),
    }
}

struct ListenHandle(u64);

struct ConnectingHandle(u64);

struct RendezvousServerState {
    listen_handle_cnt: u64,
    listen_port_pool: LinkedList<u16>,
    listeners: HashMap<u64, (Uuid, u16, Arc<TcpListener>)>,
    connecting_streams_cnt: u64,
    connecting_streams: HashMap<u64, (String, TcpStream)>,
}

struct RendezvousServer {
    listen_addr: IpAddr,
    public_hostname: String,
    rendezvous_endpoint_base: String,
    state: Mutex<RendezvousServerState>,
}

impl RendezvousServer {
    pub fn new(
        listen_addr: IpAddr,
        public_hostname: String,
        rendezvous_endpoint_base: String,
        listen_ports: Range<u16>,
    ) -> Arc<Self> {
        use rand::seq::SliceRandom;
        let mut listen_port_pool_vec: Vec<u16> = listen_ports.into_iter().collect();
        listen_port_pool_vec.shuffle(&mut rand::thread_rng());
        let listen_port_pool = listen_port_pool_vec.into_iter().collect();

        Arc::new(RendezvousServer {
            listen_addr,
            public_hostname,
            rendezvous_endpoint_base,
            state: Mutex::new(RendezvousServerState {
                listen_handle_cnt: 0,
                listen_port_pool,
                listeners: HashMap::new(),
                connecting_streams_cnt: 0,
                connecting_streams: HashMap::new(),
            }),
        })
    }

    pub async fn listen(&self, listener_id: Uuid) -> Result<Option<ListenHandle>> {
        let mut state = self.state.lock().await;

        // Get a port, start the listen task and insert a listener entry:
        let port = match state.listen_port_pool.pop_front() {
            Some(port) => port,
            None => return Ok(None),
        };

        // Try to bind to this port. If this fails, return an error
        // synchronously (as the return value of this function):
        let socket = match (self.listen_addr.is_ipv4(), self.listen_addr.is_ipv6()) {
            (true, false) => TcpSocket::new_v4(),
            (false, true) => TcpSocket::new_v6(),
            _ => panic!("Unsupported address family: {:?}", self.listen_addr),
        }
        .with_context(|| {
            format!(
                "Creating a new TcpSocket with address family from {:?}",
                self.listen_addr
            )
        })?;

        socket.set_reuseaddr(true).unwrap();
        socket.set_reuseport(true).unwrap();

        let listen_addr = SocketAddr::new(self.listen_addr, port);
        socket
            .bind(listen_addr)
            .with_context(|| format!("Binding to address {:?}", listen_addr))?;

        // TODO: make connection backlog configurable
        let listener = socket
            .listen(1024)
            .with_context(|| format!("Listening on address {:?}", listen_addr))?;

        let handle_id = state.listen_handle_cnt;
        state.listen_handle_cnt = state
            .listen_handle_cnt
            .checked_add(1)
            .expect("Invariant violated: handle count overflow");

        state
            .listeners
            .insert(handle_id, (listener_id, port, Arc::new(listener)));

        Ok(Some(ListenHandle(handle_id)))
    }

    pub async fn accept(&self, handle: &ListenHandle) -> Result<(ConnectingHandle, SocketAddr)> {
        let listener = {
            let state = self.state.lock().await;
            let (_, _, ref listener_ref) = state.listeners.get(&handle.0).expect(&format!(
                "Listener entry for handle {:?} not found.",
                handle.0
            ));
            listener_ref.clone()
        };

        let (tcp_stream, socket_addr) = listener
            .accept()
            .await
            .with_context(|| format!("Accepting socket connection for handle {:?}", handle.0))?;

        // Connection accepted! We store this in the state and hand out a handle:
        let mut state = self.state.lock().await;
        let connecting_handle_id = state.connecting_streams_cnt;
        state.connecting_streams_cnt = state
            .connecting_streams_cnt
            .checked_add(1)
            .expect("Invariant violated: connecting handle count overflow");

        // TODO: generate token to be verified on accept.
        state
            .connecting_streams
            .insert(connecting_handle_id, ("".to_string(), tcp_stream));

        Ok((ConnectingHandle(connecting_handle_id), socket_addr))
    }

    pub async fn connect(&self, handle_id: u64) -> Option<TcpStream> {
        // TODO: validate token!
        let mut state = self.state.lock().await;
        let (_, tcp_stream) = state.connecting_streams.remove(&handle_id)?;
        Some(tcp_stream)
    }

    pub async fn build_listening_resp(&self, handle: &ListenHandle) -> listener_sse_api::Listening {
        let state = self.state.lock().await;
        let (_, ref port, _) = state.listeners.get(&handle.0).expect(&format!(
            "Listener entry for handle {:?} not found.",
            handle.0
        ));

        listener_sse_api::Listening {
            public_hostname: self.public_hostname.clone(),
            public_port: *port,
        }
    }

    pub async fn build_new_connection_resp(
        &self,
        handle: &ConnectingHandle,
    ) -> listener_sse_api::NewConnection {
        let state = self.state.lock().await;
        let (ref token, ref tcp_stream) = state.connecting_streams.get(&handle.0).expect(&format!(
            "Connecting stream entry for handle {:?} not found.",
            handle.0
        ));

        let local_addr = tcp_stream
            .local_addr()
            .expect("Unable to get local address of bound connection");
        let peer_addr = tcp_stream
            .peer_addr()
            .expect("Unable to get peer address of bound connection");

        listener_sse_api::NewConnection {
            client_ip: format!("{}", peer_addr.ip()),
            client_port: peer_addr.port(),
            remote_ip: format!("{}", local_addr.ip()),
            remote_port: local_addr.port(),
            rendezvous_endpoint: format!("{}/{}", self.rendezvous_endpoint_base, handle.0),
            accept_token: token.clone(),
        }
    }
}

#[get("/listen/<listener_id>")]
fn listen(listener_id: Uuid, state: &State<Arc<RendezvousServer>>) -> EventStream![Event + '_] {
    EventStream! {
    // TODO: proper error handling
    let listen_handle = state.listen(listener_id).await.expect("Failed to listen").expect("No port available");
    yield Event::data(serde_json::to_string(&listener_sse_api::SSEMessage::Listening(state.build_listening_resp(&listen_handle).await)).unwrap());
    println!("Listening!");
        loop {
            let (connecting_handle, _) = state.accept(&listen_handle).await.expect("Failed to accept connection");
            println!("Accepted -> Connecting!");
        yield Event::data(serde_json::to_string(&listener_sse_api::SSEMessage::NewConnection(state.build_new_connection_resp(&connecting_handle).await)).unwrap());

        }
    }
}

#[get("/accept/<connecting_handle_id>")]
async fn accept(
    connecting_handle_id: u64,
    ws: ws::WebSocket,
    state: &State<Arc<RendezvousServer>>,
) -> ws::Stream!['static] {
    use rocket::futures::StreamExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let ws = ws.config(ws::Config {
        ..Default::default()
    });

    println!(
        "Accepting WS connection, handle id: {}!",
        connecting_handle_id
    );
    let mut tcp_stream = state
        .connect(connecting_handle_id)
        .await
        .expect("Stream not connecting!");

    let mut buf = vec![0; 64 * 1024];

    ws::Stream! { ws => {
    let mut ws = ws;
    loop {
        // TODO: make full duplex
            let resp = tokio::select! {
        message = ws.next() => {
            match message {
            Some(Ok(ws::Message::Binary(vec))) => {
                tcp_stream.write(&vec).await.unwrap();
            }

            None => {
                println!("Websocket listener disconnected!");
                tcp_stream.shutdown().await.unwrap();
                break;
            },
                Some(Err(e)) => {
                println!("Websocket listener errored: {:?}!", e);
                tcp_stream.shutdown().await.unwrap();
                break;
            }

            _ => {
                println!("Received unknown WS message: {:?}", message);
            }
            }

            None
        }

        size_res = tcp_stream.read(&mut buf) => {
            let size = size_res.unwrap();
            Some(ws::Message::Binary(buf[..size].to_vec()))
        }
            };

        if let Some(r) = resp {
        // TODO: no panic!
        yield r;
        }
    }
    } }
}

/// TCP <-> WebSocket Rendezvous Server
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Public hostname for incoming TCP connections
    #[arg(short = 'p', long)]
    public_hostname: String,

    /// WebSocket server base url, including the `/accept` path (e.g.,
    /// ws://localhost:8000/accept)
    #[arg(short = 'w', long)]
    websocket_base_url: String,

    /// TCP port range start
    #[arg(short = 's')]
    tcp_port_range_start: u16,

    /// TCP port range end
    #[arg(short = 'e')]
    tcp_port_range_end: u16,
}

#[launch]
fn rocket() -> _ {
    use simplelog::{
        ColorChoice, Config as SimpleLogConfig, LevelFilter, TermLogger, TerminalMode,
    };

    let args = Args::parse();

    // TODO: get log level from command line
    TermLogger::init(
        LevelFilter::Debug,
        SimpleLogConfig::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )
    .unwrap();

    let state: Arc<RendezvousServer> = RendezvousServer::new(
        IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
        args.public_hostname,
        args.websocket_base_url,
        Range {
            start: args.tcp_port_range_start,
            end: args.tcp_port_range_end,
        },
    );

    rocket::build()
        .manage(state)
        .mount("/", routes![listen, accept])
}
