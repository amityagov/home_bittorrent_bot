pub trait ResultExt<T, E> {
    fn log_error(self) -> Result<T, E>;
}

impl<T, E> ResultExt<T, E> for Result<T, E>
where
    E: std::fmt::Debug,
{
    fn log_error(self) -> Result<T, E> {
        match self {
            Ok(value) => Ok(value),
            Err(err) => {
                log::error!("{:?}", err);
                Err(err)
            }
        }
    }
}

use crate::Configuration;
use log::info;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub fn get_gateway_address() -> std::io::Result<Option<String>> {
    let file = File::open("/proc/net/route")?;
    let reader = BufReader::new(file);

    for line in reader.lines().skip(1) {
        let line = line?;
        let parts: Vec<&str> = line.split_whitespace().collect();

        // Check if this is the default route (destination is "00000000")
        if parts[1] == "00000000" {
            // The gateway address is in hex, need to reverse the byte order
            let gateway_hex = parts[2];
            if let Ok(gateway) = u32::from_str_radix(gateway_hex, 16) {
                let gateway_ip = std::net::Ipv4Addr::new(
                    (gateway & 0xFF) as u8,
                    ((gateway >> 8) & 0xFF) as u8,
                    ((gateway >> 16) & 0xFF) as u8,
                    ((gateway >> 24) & 0xFF) as u8,
                );

                info!("Default gateway: {}", gateway_ip);

                return Ok(Some(gateway_ip.to_string()));
            }
            break;
        }
    }

    Ok(None)
}

pub fn run_in_docker() -> bool {
    ["/.dockerenv", "/run/.dockerenv"]
        .iter()
        .any(|x| Path::new(x).exists())
}

pub fn get_bittorrent_api_url(configuration: &Configuration) -> anyhow::Result<String> {
    if let Some(url) = &configuration.url {
        info!("using provided address from config {}", url);
        return Ok(url.clone());
    }

    if run_in_docker() {
        let gateway_address = get_gateway_address()?;
        if let Some(gateway_address) = gateway_address {
            info!("using host ip address {}", gateway_address);
            return Ok(format!("http://{}:8080/", gateway_address));
        }
    }

    Err(anyhow::anyhow!("Failed to detect url"))
}
