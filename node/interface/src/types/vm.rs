use serde::{Deserialize, Serialize};

use super::error::ConversionError;
use crate::proto::enclave;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TcpAddress {
    pub host: String,
    pub port: u32,
}

impl From<enclave::TcpAddress> for TcpAddress {
    fn from(p: enclave::TcpAddress) -> Self {
        Self {
            host: p.host,
            port: p.port,
        }
    }
}

impl From<TcpAddress> for enclave::TcpAddress {
    fn from(value: TcpAddress) -> Self {
        enclave::TcpAddress {
            host: value.host,
            port: value.port,
        }
    }
}

impl From<&TcpAddress> for enclave::TcpAddress {
    fn from(value: &TcpAddress) -> Self {
        enclave::TcpAddress {
            host: value.host.clone(),
            port: value.port,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UdsAddress {
    pub path: String,
}

impl From<enclave::UdsAddress> for UdsAddress {
    fn from(p: enclave::UdsAddress) -> Self {
        Self { path: p.path }
    }
}

impl From<UdsAddress> for enclave::UdsAddress {
    fn from(value: UdsAddress) -> Self {
        enclave::UdsAddress { path: value.path }
    }
}

impl From<&UdsAddress> for enclave::UdsAddress {
    fn from(value: &UdsAddress) -> Self {
        enclave::UdsAddress {
            path: value.path.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VsockAddress {
    pub cid: u32,
    pub port: u32,
}

impl From<enclave::VsockAddress> for VsockAddress {
    fn from(p: enclave::VsockAddress) -> Self {
        Self {
            cid: p.cid,
            port: p.port,
        }
    }
}

impl From<VsockAddress> for enclave::VsockAddress {
    fn from(value: VsockAddress) -> Self {
        enclave::VsockAddress {
            cid: value.cid,
            port: value.port,
        }
    }
}

impl From<&VsockAddress> for enclave::VsockAddress {
    fn from(value: &VsockAddress) -> Self {
        enclave::VsockAddress {
            cid: value.cid,
            port: value.port,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VmAddress {
    Tcp(TcpAddress),
    Uds(UdsAddress),
    Vsock(VsockAddress),
}

impl TryFrom<enclave::VmAddress> for VmAddress {
    type Error = ConversionError;
    fn try_from(p: enclave::VmAddress) -> Result<Self, Self::Error> {
        match p.address_type {
            Some(enclave::vm_address::AddressType::Tcp(tcp)) => {
                Ok(VmAddress::Tcp(TcpAddress::from(tcp)))
            }
            Some(enclave::vm_address::AddressType::Uds(uds)) => {
                Ok(VmAddress::Uds(UdsAddress::from(uds)))
            }
            Some(enclave::vm_address::AddressType::Vsock(vsock)) => {
                Ok(VmAddress::Vsock(VsockAddress::from(vsock)))
            }
            None => Err(ConversionError::MissingField("address_type".to_string())),
        }
    }
}

impl From<VmAddress> for enclave::VmAddress {
    fn from(value: VmAddress) -> Self {
        let address_type = match value {
            VmAddress::Tcp(tcp) => enclave::vm_address::AddressType::Tcp(tcp.into()),
            VmAddress::Uds(uds) => enclave::vm_address::AddressType::Uds(uds.into()),
            VmAddress::Vsock(vsock) => enclave::vm_address::AddressType::Vsock(vsock.into()),
        };
        enclave::VmAddress {
            address_type: Some(address_type),
        }
    }
}

impl From<&VmAddress> for enclave::VmAddress {
    fn from(value: &VmAddress) -> Self {
        let address_type = match value {
            VmAddress::Tcp(tcp) => enclave::vm_address::AddressType::Tcp(tcp.into()),
            VmAddress::Uds(uds) => enclave::vm_address::AddressType::Uds(uds.into()),
            VmAddress::Vsock(vsock) => enclave::vm_address::AddressType::Vsock(vsock.into()),
        };
        enclave::VmAddress {
            address_type: Some(address_type),
        }
    }
}
