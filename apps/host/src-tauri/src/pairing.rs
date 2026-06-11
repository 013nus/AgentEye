use std::{
    net::{IpAddr, Ipv4Addr, UdpSocket},
    time::Duration,
};

use chrono::Utc;
use serde::Serialize;

use crate::phone_dev_server;

const PHONE_DEV_PORT: u16 = 1421;
const STATE_SERVER_PORT: u16 = 17891;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingConfig {
    host_ip: String,
    phone_url: String,
    phone_https_url: String,
    phone_http_url: String,
    phone_dev_port: u16,
    state_port: u16,
    phone_server_ready: bool,
    detected_at: String,
}

#[tauri::command]
pub async fn prepare_pairing_config(app: tauri::AppHandle) -> Result<PairingConfig, String> {
    if let Err(error) = phone_dev_server::ensure_phone_dev_server(&app).await {
        tracing::warn!("prepare pairing phone server: {error}");
    }
    Ok(get_pairing_config())
}

#[tauri::command]
pub fn get_pairing_config() -> PairingConfig {
    let host_ip = std::env::var("AGENTEYE_HOST_IP")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(detect_lan_ip)
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let phone_server_ready = phone_dev_server::is_port_ready(PHONE_DEV_PORT);
    let phone_https_url = format!("https://{host_ip}:{PHONE_DEV_PORT}/?host={host_ip}");
    let phone_http_url = format!("http://{host_ip}:{STATE_SERVER_PORT}/phone?host={host_ip}");
    let phone_url = phone_https_url.clone();

    PairingConfig {
        phone_url,
        phone_https_url,
        phone_http_url,
        host_ip,
        phone_dev_port: PHONE_DEV_PORT,
        state_port: STATE_SERVER_PORT,
        phone_server_ready,
        detected_at: Utc::now().to_rfc3339(),
    }
}

fn detect_lan_ip() -> Option<String> {
    // UDP connect 不会真正建立连接，但 Windows 会据此选择默认出网网卡。
    // 在手机热点、校园网、多网卡场景下，这比枚举第一块网卡更贴近手机实际能访问的地址。
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    let _ = socket.set_read_timeout(Some(Duration::from_millis(300)));
    let _ = socket.set_write_timeout(Some(Duration::from_millis(300)));
    socket.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).ok()?;

    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if is_lan_candidate(ip) => Some(ip.to_string()),
        _ => None,
    }
}

fn is_lan_candidate(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    !ip.is_loopback() && !(octets[0] == 169 && octets[1] == 254) && !ip.is_unspecified()
}
