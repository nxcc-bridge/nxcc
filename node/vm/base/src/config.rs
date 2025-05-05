use std::{net::SocketAddr, str::FromStr};

use clap::{Args, Parser};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Error that can occur when converting from VmServerConfig to ServerConfig
#[derive(Error, Debug)]
pub enum ConfigConversionError {
    #[error(
        "Feature '{0}' is required for server mode '{1}' but was not enabled during compilation"
    )]
    FeatureNotEnabled(String, String),

    #[error("Invalid socket address: {0}")]
    InvalidSocketAddr(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
}

/// Base configuration for VM services
#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
#[command(author, version, about, long_about = None)]
#[group(skip)]
#[derive(Default)]
pub struct Config {
    /// Enable verbose logging
    #[arg(short, long, default_value_t = false)]
    pub verbose: bool,

    /// Server configuration
    #[serde(default)]
    #[clap(flatten)]
    pub server: VmServerConfig,
}

/// Configuration for the VM server
#[derive(Args, Debug, Clone, Serialize, Deserialize)]
pub struct VmServerConfig {
    /// Server mode: "uds", "vsock", or "tcp"
    #[clap(long = "server-mode", default_value_t = default_server_mode())]
    #[serde(default = "default_server_mode")]
    pub mode: String,

    /// When using UDS: the path to the Unix Domain Socket
    #[clap(long = "server-uds-path", default_value_t = default_uds_path())]
    #[serde(default = "default_uds_path")]
    pub uds_path: String,

    /// When using vsock: the vsock CID to listen on
    #[clap(long = "server-vsock-cid", default_value_t = default_vsock_cid())]
    #[serde(default = "default_vsock_cid")]
    pub vsock_cid: u32,

    /// When using vsock: the vsock port to listen on
    #[clap(long = "server-vsock-port", default_value_t = default_vsock_port())]
    #[serde(default = "default_vsock_port")]
    pub vsock_port: u32,

    /// When using TCP: the address to listen on (e.g., "127.0.0.1:8080")
    #[clap(long = "server-tcp-addr", default_value_t = default_tcp_addr())]
    #[serde(default = "default_tcp_addr")]
    pub tcp_addr: String,
}

impl Default for VmServerConfig {
    fn default() -> Self {
        Self {
            mode: default_server_mode(),
            uds_path: default_uds_path(),
            vsock_cid: default_vsock_cid(),
            vsock_port: default_vsock_port(),
            tcp_addr: default_tcp_addr(),
        }
    }
}

// Default value functions
fn default_server_mode() -> String {
    #[cfg(feature = "uds")]
    return "uds".to_string();

    #[cfg(all(not(feature = "uds"), feature = "vsock"))]
    return "vsock".to_string();

    #[cfg(all(not(feature = "uds"), not(feature = "vsock"), feature = "tcp"))]
    return "tcp".to_string();

    #[cfg(not(any(feature = "uds", feature = "vsock", feature = "tcp")))]
    return "none".to_string();
}

fn default_uds_path() -> String {
    "/tmp/vm_service.sock".to_string()
}

fn default_vsock_cid() -> u32 {
    3 // VMADDR_CID_ANY
}

fn default_vsock_port() -> u32 {
    5000
}

fn default_tcp_addr() -> String {
    "127.0.0.1:8080".to_string()
}

impl TryFrom<&VmServerConfig> for crate::server::ServerConfig {
    type Error = ConfigConversionError;

    fn try_from(config: &VmServerConfig) -> Result<Self, Self::Error> {
        match config.mode.as_str() {
            "uds" => {
                #[cfg(feature = "uds")]
                {
                    Ok(crate::server::ServerConfig::Uds {
                        path: config.uds_path.clone(),
                    })
                }
                #[cfg(not(feature = "uds"))]
                {
                    Err(ConfigConversionError::FeatureNotEnabled(
                        "uds".to_string(),
                        "uds".to_string(),
                    ))
                }
            }
            "vsock" => {
                #[cfg(feature = "vsock")]
                {
                    Ok(crate::server::ServerConfig::Vsock {
                        cid: config.vsock_cid,
                        port: config.vsock_port,
                    })
                }
                #[cfg(not(feature = "vsock"))]
                {
                    Err(ConfigConversionError::FeatureNotEnabled(
                        "vsock".to_string(),
                        "vsock".to_string(),
                    ))
                }
            }
            "tcp" => {
                #[cfg(feature = "tcp")]
                {
                    let addr = SocketAddr::from_str(&config.tcp_addr)
                        .map_err(|e| ConfigConversionError::InvalidSocketAddr(e.to_string()))?;
                    Ok(crate::server::ServerConfig::Tcp { addr })
                }
                #[cfg(not(feature = "tcp"))]
                {
                    Err(ConfigConversionError::FeatureNotEnabled(
                        "tcp".to_string(),
                        "tcp".to_string(),
                    ))
                }
            }
            _ => Err(ConfigConversionError::InvalidConfig(format!(
                "Unsupported server mode: {}",
                config.mode
            ))),
        }
    }
}
