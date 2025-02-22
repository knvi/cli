#[cfg(windows)]
use std::env::temp_dir;
use std::net::IpAddr;
use std::path::PathBuf;
#[cfg(not(windows))]
use std::sync::Arc;

use anyhow::{anyhow, Result};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
#[cfg(windows)]
use tokio_native_tls::{native_tls::TlsConnector, TlsStream};
#[cfg(not(windows))]
use tokio_rustls::{
    client::TlsStream,
    rustls::{ClientConfig, OwnedTrustAnchor, RootCertStore},
};

use super::types::{Prefix, TonneruPacket};
use super::{TONNERU_PORT, TONNERU_URI};
use crate::commands::update::util::execute_commands;
use crate::utils::is_writable;

#[derive(Clone)]
pub struct TonneruSocket {
    token: String,
    resource_id: String,
    port: u16,
    #[cfg(windows)]
    pub config: TlsConnector,
    #[cfg(not(windows))]
    pub config: Arc<ClientConfig>,
}

type TlsSocket = TlsStream<TcpStream>;

impl TonneruSocket {
    pub fn new(token: &str, resource_id: &str, port: u16) -> Result<Self> {
        #[cfg(windows)]
        let config = native_tls::TlsConnector::new()?;

        #[cfg(not(windows))]
        let config = {
            // ref: https://github.com/rustls/hyper-rustls/blob/main/src/config.rs#L55
            let mut roots = RootCertStore::empty();
            roots.add_server_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.0.iter().map(|ta| {
                OwnedTrustAnchor::from_subject_spki_name_constraints(
                    ta.subject,
                    ta.spki,
                    ta.name_constraints,
                )
            }));

            Arc::new(
                ClientConfig::builder()
                    .with_safe_defaults()
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            )
        };

        Ok(Self {
            token: token.to_string(),
            resource_id: resource_id.to_string(),
            port,
            config,
        })
    }

    #[cfg(not(windows))]
    async fn open_socket(&self) -> Result<TlsSocket> {
        use tokio_rustls::rustls::ServerName;
        use tokio_rustls::TlsConnector;

        let remote = TcpStream::connect(format!("{TONNERU_URI}:{TONNERU_PORT}")).await?;

        log::debug!("Connected to {TONNERU_URI}:{TONNERU_PORT}");

        let dns_name = ServerName::try_from(TONNERU_URI)?;

        log::debug!("Connecting to {TONNERU_URI} with TLS");

        TlsConnector::from(self.config.clone())
            .connect(dns_name, remote)
            .await
            .map_err(|e| anyhow!("Failed to connect to {TONNERU_URI}: {e}"))
    }

    #[cfg(windows)]
    async fn open_socket(&self) -> Result<TlsSocket> {
        use tokio_native_tls::TlsConnector;

        let remote = TcpStream::connect(format!("{TONNERU_URI}:{TONNERU_PORT}")).await?;

        log::debug!("TLS connection open to {TONNERU_URI}:{TONNERU_PORT}");

        TlsConnector::from(self.config.clone())
            .connect(TONNERU_URI, remote)
            .await
            .map_err(|e| anyhow!("Failed to connect to {TONNERU_URI}: {e}"))
    }

    pub async fn connect(&self) -> Result<TlsSocket> {
        let mut socket = self.open_socket().await?;

        let packet = serde_json::to_vec(&TonneruPacket::Auth {
            token: self.token.clone(),
            resource_id: self.resource_id.clone(),
            port: self.port,
        })?;

        log::debug!(
            "Sending auth packet: {}",
            String::from_utf8_lossy(&packet).replace(&self.token, "********")
        );

        socket.write_all(&packet).await?;

        let mut buf = [0; 1024];

        match socket.read(&mut buf).await {
            Ok(n) => match serde_json::from_slice::<TonneruPacket>(&buf[..n]) {
                Ok(TonneruPacket::Connect { .. }) => {
                    log::debug!(
                        "Successfully established connection to Tonneru, forwarding traffic"
                    );

                    Ok(socket)
                }

                _ => Err(anyhow!(
                    "Unexpected packet. Received: {}",
                    String::from_utf8_lossy(&buf[..n])
                )),
            },
            Err(e) => Err(anyhow!("Failed to read from socket: {}", e)),
        }
    }
}

pub fn parse_publish(publish: &str) -> Result<(IpAddr, u16, u16)> {
    let mut split = publish.split(':');

    if split.clone().count() > 3 {
        return Err(anyhow!("Invalid port format."));
    }

    match (split.next(), split.next(), split.next()) {
        (Some(ip), Some(local), Some(external)) => {
            Ok((ip.parse()?, local.parse::<u16>()?, external.parse::<u16>()?))
        }

        (Some(local), Some(external), None) => {
            if local.contains('.') {
                let port = external.parse::<u16>()?;

                Ok((local.parse()?, port, port))
            } else {
                Ok(([127, 0, 0, 1].into(), local.parse()?, external.parse()?))
            }
        }

        (Some(port), None, None) => {
            let common = port.parse::<u16>()?;

            Ok(([127, 0, 0, 1].into(), common, common))
        }

        _ => Err(anyhow!("Invalid port format.")),
    }
}

#[cfg(not(windows))]
const SUDO_NAME: &str = "root";
#[cfg(windows)]
const SUDO_NAME: &str = "administrative";

pub async fn add_entry_to_hosts(domain: &str, address: &str) -> Result<()> {
    log::debug!("Adding entry to hosts: {domain} -> {address}");

    #[cfg(not(windows))]
    let path = PathBuf::from("/etc/hosts");

    #[cfg(windows)]
    let path = PathBuf::from("C:\\Windows\\System32\\drivers\\etc\\hosts");
    #[cfg(windows)]
    let temp_hosts = temp_dir().join(format!("hosts.{domain}.tonneru"));

    let mut hosts = fs::read_to_string(&path)
        .await?
        .trim_matches('\n')
        .to_string();

    hosts.push_str(&format!("\n{address}\t{domain}\t# Added by Hop CLI"));

    #[cfg(windows)]
    fs::write(&temp_hosts, &hosts).await?;

    #[cfg(not(windows))]
    let edit_host = format!(
        "echo '{}' | tee {} > /dev/null",
        hosts,
        path.to_str().unwrap()
    );

    #[cfg(windows)]
    let edit_host = format!(
        "copy {} {}",
        temp_hosts.to_str().unwrap(),
        path.to_str().unwrap()
    );

    let mut elevated_args = vec![];
    let mut non_elevated_args = vec![];

    if is_writable(&path).await {
        non_elevated_args.push(edit_host.into());
    } else {
        log::warn!("Adding entry to hosts requires {SUDO_NAME} permissions.");
        elevated_args.push(edit_host.into());
    };

    execute_commands(&non_elevated_args, &elevated_args).await?;

    #[cfg(windows)]
    fs::remove_file(&temp_hosts).await?;

    Ok(())
}

pub async fn remove_entry_from_hosts(domain: &str) -> Result<()> {
    #[cfg(not(windows))]
    let path = PathBuf::from("/etc/hosts");

    #[cfg(windows)]
    let path = PathBuf::from("C:\\Windows\\System32\\drivers\\etc\\hosts");

    #[cfg(windows)]
    let temp_hosts = temp_dir().join(format!("hosts.{domain}.tonneru"));

    let hosts = fs::read_to_string(&path).await?;

    let hosts = hosts
        .lines()
        .filter(|l| !l.contains(domain))
        .collect::<Vec<_>>()
        .join("\n");

    #[cfg(windows)]
    fs::write(&temp_hosts, &hosts).await?;

    #[cfg(not(windows))]
    let edit_host = format!(
        "echo '{}' | tee {} > /dev/null",
        hosts,
        path.to_str().unwrap()
    );

    #[cfg(windows)]
    let edit_host = format!(
        "copy {} {}",
        temp_hosts.to_str().unwrap(),
        path.to_str().unwrap()
    );

    let mut elevated_args = vec![];
    let mut non_elevated_args = vec![];

    if is_writable(&path).await {
        non_elevated_args.push(edit_host.into());
    } else {
        log::warn!("Removing entry from hosts requires {SUDO_NAME} permissions.");
        elevated_args.push(edit_host.into());
    };

    execute_commands(&non_elevated_args, &elevated_args).await?;

    #[cfg(windows)]
    fs::remove_file(&temp_hosts).await?;

    Ok(())
}

pub fn get_id_with_prefix(id: Option<&str>) -> Option<(Prefix, String)> {
    if let Some(id) = id {
        let mut split = id.split('_');

        Some((
            split.next().map(|p| p.parse().unwrap()).unwrap_or_default(),
            id.to_string(),
        ))
    } else {
        None
    }
}
