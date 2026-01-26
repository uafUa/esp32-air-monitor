use std::cmp::Ordering;
use std::time::Duration;

use anyhow::{anyhow, Result};
use embedded_svc::http::{client::Client as HttpClient, Method};
use embedded_svc::utils::io;
use esp_idf_svc::http::client::EspHttpConnection;
use esp_idf_svc::ota::EspOta;
use esp_idf_svc::sys::esp_restart;
use log::info;

use crate::wifi::ensure_connected;

const OTA_BASE_URL: &str = match option_env!("OTA_BASE_URL") {
    Some(v) => v,
    None => "http://192.168.1.18:8000/firmware",
};
const OTA_LATEST_FILE: &str = "latest.txt";
const OTA_FILE_PREFIX: &str = "c6-co";
const OTA_FILE_EXT: &str = ".bin";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const OTA_BUILD: Option<&str> = option_env!("OTA_BUILD");

pub const OTA_CHECK_INTERVAL: Duration = Duration::from_secs(900);

pub fn mark_app_valid() -> Result<()> {
    let mut ota = EspOta::new()?;
    ota.mark_running_slot_valid()?;
    Ok(())
}

pub fn check_and_update(wifi: &mut esp_idf_svc::wifi::BlockingWifi<esp_idf_svc::wifi::EspWifi<'static>>) -> Result<()> {
    info!("OTA check: {}", OTA_BASE_URL);
    ensure_connected(wifi)?;

    let mut client = HttpClient::wrap(EspHttpConnection::new(&Default::default())?);

    let latest_name = fetch_latest_filename(&mut client)?;
    info!("OTA latest file: {}", latest_name.trim());
    let latest_build = parse_build_from_filename(&latest_name)
        .ok_or_else(|| anyhow!("invalid OTA filename: {latest_name}"))?;
    let current_build = parse_current_build()
        .ok_or_else(|| anyhow!("invalid current build: {CURRENT_VERSION}"))?;

    if latest_build.cmp(&current_build) != Ordering::Greater {
        info!(
            "OTA up-to-date: current build {} (latest {})",
            current_build, latest_build
        );
        return Ok(());
    }

    let url = format!("{}/{}", OTA_BASE_URL.trim_end_matches('/'), latest_name.trim());
    info!(
        "OTA update available: build {} -> {}",
        current_build,
        latest_name.trim()
    );
    perform_update(&mut client, &url)?;

    // If update succeeds, the device reboots in perform_update().
    Ok(())
}

fn fetch_latest_filename(client: &mut HttpClient<EspHttpConnection>) -> Result<String> {
    let url = format!("{}/{}", OTA_BASE_URL.trim_end_matches('/'), OTA_LATEST_FILE);
    let request = client.request(Method::Get, &url, &[])?;
    let mut response = request.submit()?;

    if response.status() != 200 {
        return Err(anyhow!("OTA latest request failed: {}", response.status()));
    }

    let mut buf = [0u8; 256];
    let size = io::try_read_full(&mut response, &mut buf).map_err(|e| e.0)?;
    let text = std::str::from_utf8(&buf[..size])?;
    Ok(text.trim().to_string())
}

fn perform_update(client: &mut HttpClient<EspHttpConnection>, url: &str) -> Result<()> {
    info!("OTA download start: {}", url);
    let request = client.request(Method::Get, url, &[])?;
    let mut response = request.submit()?;

    if response.status() != 200 {
        return Err(anyhow!("OTA firmware request failed: {}", response.status()));
    }

    let mut ota = EspOta::new()?;
    let mut update = ota.initiate_update()?;
    let mut buf = [0u8; 1024];

    loop {
        let n = response.read(&mut buf)?;
        if n == 0 {
            break;
        }
        update.write(&buf[..n])?;
    }

    update.complete()?;
    info!("OTA update complete, rebooting...");
    unsafe { esp_restart() };
}

fn parse_build_from_filename(name: &str) -> Option<u32> {
    let stem = name.trim();
    let stem = stem.rsplit('/').next()?;
    let stem = stem.strip_suffix(OTA_FILE_EXT).unwrap_or(stem);
    let stem = stem.trim();
    let suffix = stem.strip_prefix(OTA_FILE_PREFIX)?;
    if suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    suffix.parse::<u32>().ok()
}

fn parse_current_build() -> Option<u32> {
    if let Some(build) = OTA_BUILD {
        return parse_build_suffix(build);
    }
    if let Some((_, build)) = CURRENT_VERSION.split_once('+') {
        return parse_build_suffix(build);
    }
    parse_build_suffix(CURRENT_VERSION)
}

fn parse_build_suffix(text: &str) -> Option<u32> {
    let digits: String = text
        .trim()
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse::<u32>().ok()
    }
}
