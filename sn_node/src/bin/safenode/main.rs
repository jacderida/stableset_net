// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

#[macro_use]
extern crate tracing;

mod rpc;
mod run;

use clap::Parser;
use eyre::Result;
use sn_logging::LogFormat;
use sn_peers_acquisition::PeersArgs;
use std::{
    env,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};
use tokio::signal::ctrl_c;

#[derive(Debug, Clone)]
pub enum LogOutputDestArg {
    Stdout,
    DataDir,
    Path(PathBuf),
}

impl std::fmt::Display for LogOutputDestArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogOutputDestArg::Stdout => write!(f, "stdout"),
            LogOutputDestArg::DataDir => write!(f, "data-dir"),
            LogOutputDestArg::Path(path) => write!(f, "{}", path.display()),
        }
    }
}

pub fn parse_log_output(val: &str) -> Result<LogOutputDestArg> {
    match val {
        "stdout" => Ok(LogOutputDestArg::Stdout),
        "data-dir" => Ok(LogOutputDestArg::DataDir),
        // The path should be a directory, but we can't use something like `is_dir` to check
        // because the path doesn't need to exist. We can create it for the user.
        value => Ok(LogOutputDestArg::Path(PathBuf::from(value))),
    }
}

// Please do not remove the blank lines in these doc comments.
// They are used for inserting line breaks when the help menu is rendered in the UI.
#[derive(Parser, Debug)]
#[clap(name = "safenode cli", version = env!("CARGO_PKG_VERSION"))]
pub struct Opt {
    /// Specify the logging output destination.
    ///
    /// Valid values are "stdout", "data-dir", or a custom path.
    ///
    /// `data-dir` is the default value.
    ///
    /// The data directory location is platform specific:
    ///  - Linux: $HOME/.local/share/safe/node/<peer-id>/logs
    ///  - macOS: $HOME/Library/Application Support/safe/node/<peer-id>/logs
    ///  - Windows: C:\Users\<username>\AppData\Roaming\safe\node\<peer-id>\logs
    #[allow(rustdoc::invalid_html_tags)]
    #[clap(long, default_value_t = LogOutputDestArg::DataDir, value_parser = parse_log_output, verbatim_doc_comment)]
    log_output_dest: LogOutputDestArg,

    /// Specify the logging format.
    ///
    /// Valid values are "default" or "json".
    ///
    /// If the argument is not used, the default format will be applied.
    #[clap(long, value_parser = LogFormat::parse_from_str, verbatim_doc_comment)]
    log_format: Option<LogFormat>,

    /// Specify the maximum number of uncompressed log files to store.
    ///
    /// This argument is ignored if `log_output_dest` is set to "stdout"
    ///
    /// After reaching this limit, the older files are archived to save space.
    /// You can also specify the maximum number of archived log files to keep.
    #[clap(long = "max_log_files", verbatim_doc_comment)]
    max_uncompressed_log_files: Option<usize>,

    /// Specify the maximum number of archived log files to store.
    ///
    /// This argument is ignored if `log_output_dest` is set to "stdout"
    ///
    /// After reaching this limit, the older archived files are deleted.
    #[clap(long = "max_archived_log_files", verbatim_doc_comment)]
    max_compressed_log_files: Option<usize>,

    /// Specify the node's data directory.
    ///
    /// If not provided, the default location is platform specific:
    ///  - Linux: $HOME/.local/share/safe/node/<peer-id>
    ///  - macOS: $HOME/Library/Application Support/safe/node/<peer-id>
    ///  - Windows: C:\Users\<username>\AppData\Roaming\safe\node\<peer-id>
    #[allow(rustdoc::invalid_html_tags)]
    #[clap(long, verbatim_doc_comment)]
    root_dir: Option<PathBuf>,

    /// Specify the port to listen on.
    ///
    /// The special value `0` will cause the OS to assign a random port.
    #[clap(long, default_value_t = 0)]
    port: u16,

    /// Specify the IP to listen on.
    ///
    /// The special value `0.0.0.0` binds to all network interfaces available.
    #[clap(long, default_value_t = IpAddr::V4(Ipv4Addr::UNSPECIFIED))]
    ip: IpAddr,

    #[command(flatten)]
    peers: PeersArgs,

    /// Enable the admin/control RPC service by providing an IP and port for it to listen on.
    ///
    /// The RPC service can be used for querying information about the running node.
    #[clap(long)]
    rpc: Option<SocketAddr>,

    /// Run the node in local mode.
    ///
    /// When this flag is set, we will not filter out local addresses that we observe.
    #[clap(long)]
    local: bool,

    #[cfg(feature = "open-metrics")]
    /// Specify the port to start the OpenMetrics Server in.
    ///
    /// The special value `0` will cause the OS to assign a random port.
    #[clap(long, default_value_t = 0)]
    metrics_server_port: u16,

    #[cfg(windows)]
    /// Set to true to indicate the node should run as a service.
    #[clap(long)]
    run_as_service: bool,
}

#[cfg(unix)]
fn main() -> Result<()> {
    color_eyre::install()?;
    let opt = Opt::parse();
    crate::run::run(opt)
}

#[cfg(windows)]
fn main() -> Result<()> {
    let opt = Opt::parse();
    if opt.run_as_service {
        windows_service::main()?;
    } else {
        color_eyre::install()?;
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel(1);

            let shutdown_tx_clone = shutdown_tx.clone();
            let ctrl_c_handle = tokio::spawn(async move {
                ctrl_c().await.unwrap();
                let _ = shutdown_tx_clone.send(());
            });

            if let Err(e) = crate::run::run(opt, shutdown_tx).await {
                error!("Node error: {}", e);
            }

            tokio::select! {
                _ = ctrl_c_handle => {
                    // Ctrl+C handle finished execution
                },
                _ = shutdown_rx.recv() => {
                    // Got shutdown signal
                }
            }
        });
    }
    Ok(())
}

#[cfg(windows)]
mod windows_service {
    use super::Opt;
    use clap::Parser;
    use std::ffi::OsString;
    use tokio::sync::mpsc;
    use tokio::time::Duration;
    use tracing::error;
    use windows_service::{
        define_windows_service,
        service::{
            ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
            ServiceType,
        },
        service_control_handler::{self, ServiceControlHandlerResult},
        service_dispatcher, Result,
    };

    const SERVICE_NAME: &str = "safenode4";
    const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

    define_windows_service!(ffi_service_main, node_service_main);

    fn node_service_main(args: Vec<OsString>) {
        let opt = Opt::parse_from(args.iter());
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            if let Err(e) = run_service(opt).await {
                error!("Service encountered error: {}", e);
            }
        });
    }

    pub fn main() -> std::io::Result<()> {
        service_dispatcher::start(SERVICE_NAME, ffi_service_main)
            .map_err(|x| std::io::Error::new(std::io::ErrorKind::Other, x))
    }

    async fn run_service(opt: Opt) -> Result<()> {
        // Create a channel to be able to poll a stop event from the service worker loop.
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);

        let event_handler = {
            let shutdown_tx = shutdown_tx.clone();
            move |control_event| -> ServiceControlHandlerResult {
                match control_event {
                    ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                    ServiceControl::Stop => match shutdown_tx.try_send(()) {
                        Ok(_) => ServiceControlHandlerResult::NoError,
                        Err(err) => {
                            error!("Failed to send shutdown signal: {}", err);
                            ServiceControlHandlerResult::Other(1)
                        }
                    },
                    _ => ServiceControlHandlerResult::NotImplemented,
                }
            }
        };

        let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;
        status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })?;

        let shutdown_tx_clone = shutdown_tx.clone();
        let mut handle = Some(tokio::spawn(async move {
            if let Err(x) = crate::run::run(opt, shutdown_tx_clone).await {
                error!("Error: {x}");
            }
        }));

        loop {
            tokio::select! {
                _ = handle.take().unwrap() => {
                    debug!("Main run_node thread finished executing");
                    break;
                },
                _ = shutdown_rx.recv() => {
                    debug!("Received shutdown signal");
                    break;
                },
                _ = tokio::time::sleep(Duration::from_millis(100)) => {},
            }
        }

        status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::Stopped,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })?;

        Ok(())
    }
}
