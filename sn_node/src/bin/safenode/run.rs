use crate::{LogOutputDestArg, Opt};
use eyre::{eyre, Error, Result};
use libp2p::{identity::Keypair, PeerId};
#[cfg(feature = "metrics")]
use sn_logging::metrics::init_metrics;
use sn_logging::{LogFormat, LogOutputDest};
use sn_node::{Marker, NodeBuilder, NodeEvent};
use sn_peers_acquisition::parse_peers_args;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio::sync::broadcast::error::RecvError;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_core::Level;

pub async fn run(opt: Opt, shutdown_tx: tokio::sync::mpsc::Sender<()>) -> Result<()> {
    let node_socket_addr = SocketAddr::new(opt.ip, opt.port);
    let (root_dir, keypair) = get_root_dir_and_keypair(&opt.root_dir)?;

    let (log_output_dest, log_appender_guard) =
        init_logging(&opt, keypair.public().to_peer_id()).await?;

    // bootstrap peers can be empty for the genesis node.
    let bootstrap_peers = parse_peers_args(opt.peers).await.unwrap_or(vec![]);
    let msg = format!(
        "Running {} v{}",
        env!("CARGO_BIN_NAME"),
        env!("CARGO_PKG_VERSION")
    );
    info!("\n{}\n{}", msg, "=".repeat(msg.len()));
    debug!("Built with git version: {}", sn_build_info::git_info());

    info!("Node started with initial_peers {bootstrap_peers:?}");

    #[cfg(feature = "metrics")]
    tokio::spawn(init_metrics(std::process::id()));

    let node_builder = NodeBuilder::new(
        keypair,
        node_socket_addr,
        bootstrap_peers,
        opt.local,
        root_dir,
    );

    #[cfg(feature = "open-metrics")]
    let mut node_builder = node_builder;
    #[cfg(feature = "open-metrics")]
    node_builder.metrics_server_port(opt.metrics_server_port);

    let started_instant = std::time::Instant::now();

    info!("Starting node ...");
    let running_node = node_builder.build_and_run()?;

    println!(
        "
Node started

PeerId is {}
You can check your reward balance by running:
`safe wallet balance --peer-id={}`
    ",
        running_node.peer_id(),
        running_node.peer_id()
    );

    // write the PID to the root dir
    let pid = std::process::id();
    let pid_file = running_node.root_dir_path().join("safenode.pid");
    std::fs::write(pid_file, pid.to_string().as_bytes())?;

    // Monitor `NodeEvents`
    let mut node_events_rx = running_node.node_events_channel().subscribe();

    // Start up gRPC interface if enabled by user
    if let Some(addr) = opt.rpc {
        crate::rpc::start_rpc_service(
            addr,
            &log_output_dest,
            running_node.clone(),
            started_instant,
        );
    }

    loop {
        match node_events_rx.recv().await {
            Ok(NodeEvent::ConnectedToNetwork) => Marker::NodeConnectedToNetwork.log(),
            Ok(NodeEvent::ChannelClosed) | Err(RecvError::Closed) => {
                if let Err(err) = shutdown_tx.send(()).await {
                    error!("Failed to send node control msg to safenode bin main thread: {err}");
                    break;
                }
            }
            Ok(NodeEvent::BehindNat) => {
                if let Err(err) = shutdown_tx.send(()).await {
                    error!("Failed to send node control msg to safenode bin main thread: {err}");
                    break;
                }
            }
            Ok(event) => {
                /* we ignore other events */
                info!("Currently ignored node event {event:?}");
            }
            Err(RecvError::Lagged(n)) => {
                warn!("Skipped {n} node events!");
                continue;
            }
        }
    }

    drop(log_appender_guard);

    Ok(())
}

/// The keypair is located inside the root directory. At the same time, when no dir is specified,
/// the dir name is derived from the keypair used in the application: the peer ID is used as the directory name.
fn get_root_dir_and_keypair(root_dir: &Option<PathBuf>) -> Result<(PathBuf, Keypair)> {
    match root_dir {
        Some(dir) => {
            std::fs::create_dir_all(dir)?;

            let secret_key_path = dir.join("secret-key");
            Ok((dir.clone(), keypair_from_path(secret_key_path)?))
        }
        None => {
            let secret_key = libp2p::identity::ed25519::SecretKey::generate();
            let keypair: Keypair =
                libp2p::identity::ed25519::Keypair::from(secret_key.clone()).into();
            let peer_id = keypair.public().to_peer_id();

            let dir = get_root_dir(peer_id)?;
            std::fs::create_dir_all(&dir)?;

            let secret_key_path = dir.join("secret-key");

            let mut file = create_secret_key_file(secret_key_path)
                .map_err(|err| eyre!("could not create secret key file: {err}"))?;
            file.write_all(secret_key.as_ref())?;

            Ok((dir, keypair))
        }
    }
}

async fn init_logging(opt: &Opt, peer_id: PeerId) -> Result<(String, Option<WorkerGuard>)> {
    let logging_targets = vec![
        // TODO: Reset to nice and clean defaults once we have a better idea of what we want
        ("sn_networking".to_string(), Level::DEBUG),
        ("safenode".to_string(), Level::TRACE),
        ("sn_build_info".to_string(), Level::TRACE),
        ("sn_logging".to_string(), Level::TRACE),
        ("sn_node".to_string(), Level::TRACE),
        ("sn_peers_acquisition".to_string(), Level::TRACE),
        ("sn_protocol".to_string(), Level::TRACE),
        ("sn_registers".to_string(), Level::TRACE),
        ("sn_testnet".to_string(), Level::TRACE),
        ("sn_transfers".to_string(), Level::TRACE),
    ];

    let output_dest = match &opt.log_output_dest {
        LogOutputDestArg::Stdout => LogOutputDest::Stdout,
        LogOutputDestArg::DataDir => {
            let path = get_root_dir(peer_id)?.join("logs");
            LogOutputDest::Path(path)
        }
        LogOutputDestArg::Path(path) => LogOutputDest::Path(path.clone()),
    };

    #[cfg(not(feature = "otlp"))]
    let log_appender_guard = {
        let mut log_builder = sn_logging::LogBuilder::new(logging_targets);
        log_builder.output_dest(output_dest.clone());
        log_builder.format(opt.log_format.clone().unwrap_or(LogFormat::Default));
        if let Some(files) = opt.max_uncompressed_log_files {
            log_builder.max_uncompressed_log_files(files);
        }
        if let Some(files) = opt.max_compressed_log_files {
            log_builder.max_compressed_log_files(files);
        }

        log_builder.initialize()?
    };

    #[cfg(feature = "otlp")]
    let log_appender_guard = {
        // This 'oneshot' channel is used to get the guard back from the spawned thread.
        // The copying of data is necessary to access it in the new thread without compiler
        // errors.
        let (tx, rx) = tokio::sync::oneshot::channel();
        let output_dest_clone = output_dest.clone();
        let log_format = opt.log_format.clone().unwrap_or(LogFormat::Default);
        let max_uncompressed_log_files = opt.max_uncompressed_log_files.clone();
        let max_compressed_log_files = opt.max_compressed_log_files.clone();

        // init logging in a separate thread if we are sending traces to an opentelemetry server
        tokio::spawn(async move {
            let mut log_builder = sn_logging::LogBuilder::new(logging_targets);
            log_builder.output_dest(output_dest_clone.clone());
            log_builder.format(log_format);
            if let Some(files) = max_uncompressed_log_files {
                log_builder.max_uncompressed_log_files(files);
            }
            if let Some(files) = max_compressed_log_files {
                log_builder.max_compressed_log_files(files);
            }
            let guard = log_builder.initialize();
            tx.send(guard).unwrap();
        });
        let guard = rx.await.unwrap()?;
        guard
    };

    Ok((output_dest.to_string(), log_appender_guard))
}

fn get_root_dir(peer_id: PeerId) -> Result<PathBuf> {
    let dir = dirs_next::data_dir()
        .ok_or_else(|| eyre!("could not obtain root directory path".to_string()))?
        .join("safe")
        .join("node")
        .join(peer_id.to_string());
    Ok(dir)
}

fn create_secret_key_file(path: impl AsRef<Path>) -> Result<std::fs::File, std::io::Error> {
    let mut opt = std::fs::OpenOptions::new();
    opt.write(true).create_new(true);

    // On Unix systems, make sure only the current user can read/write.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opt.mode(0o600);
    }

    opt.open(path)
}

fn keypair_from_path(path: impl AsRef<Path>) -> Result<Keypair> {
    let keypair = match std::fs::read(&path) {
        // If the file is opened successfully, read the key from it
        Ok(key) => {
            let keypair = Keypair::ed25519_from_bytes(key)
                .map_err(|err| eyre!("could not read ed25519 key from file: {err}"))?;

            info!("loaded secret key from file: {:?}", path.as_ref());

            keypair
        }
        // In case the file is not found, generate a new keypair and write it to the file
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let secret_key = libp2p::identity::ed25519::SecretKey::generate();
            let mut file = create_secret_key_file(&path)
                .map_err(|err| eyre!("could not create secret key file: {err}"))?;
            file.write_all(secret_key.as_ref())?;

            info!("generated new key and stored to file: {:?}", path.as_ref());

            libp2p::identity::ed25519::Keypair::from(secret_key).into()
        }
        // Else the file can't be opened, for whatever reason (e.g. permissions).
        Err(err) => {
            return Err(eyre!("failed to read secret key file: {err}"));
        }
    };

    Ok(keypair)
}

/// Starts a new process running the binary with the same args as
/// the current process
fn start_new_node_process() {
    // Retrieve the current executable's path
    let current_exe = std::env::current_exe().unwrap();

    // Retrieve the command-line arguments passed to this process
    let args: Vec<String> = std::env::args().collect();

    info!("Original args are: {args:?}");

    // Create a new Command instance to run the current executable
    let mut cmd = Command::new(current_exe);

    // Set the arguments for the new Command
    cmd.args(&args[1..]); // Exclude the first argument (binary path)

    warn!(
        "Attempting to start a new process as node process loop has been broken: {:?}",
        cmd
    );
    // Execute the command
    let _handle = match cmd.spawn() {
        Ok(status) => status,
        Err(e) => {
            // Do not return an error as this isn't a critical failure.
            // The current node can continue.
            eprintln!("Failed to execute hard-restart command: {}", e);
            error!("Failed to execute hard-restart command: {}", e);

            return;
        }
    };
}
