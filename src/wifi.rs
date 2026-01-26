use anyhow::{anyhow, Result};
use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};
use esp_idf_hal::modem::Modem;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};

const WIFI_SSID: &str = match option_env!("WIFI_SSID") {
    Some(v) => v,
    None => "uaf-wifi2",
};
const WIFI_PASS: &str = match option_env!("WIFI_PASS") {
    Some(v) => v,
    None => "HalfLife2",
};

pub fn init_wifi(modem: Modem) -> Result<BlockingWifi<EspWifi<'static>>> {
    if WIFI_SSID == "YOUR_WIFI_SSID" {
        return Err(anyhow!("WIFI_SSID not configured"));
    }
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(modem, sys_loop.clone(), Some(nvs))?,
        sys_loop,
    )?;

    let auth_method = if WIFI_PASS.is_empty() {
        AuthMethod::None
    } else {
        AuthMethod::WPA2Personal
    };

    let ssid = WIFI_SSID
        .try_into()
        .map_err(|_| anyhow!("WIFI_SSID too long"))?;
    let password = WIFI_PASS
        .try_into()
        .map_err(|_| anyhow!("WIFI_PASS too long"))?;

    let cfg = Configuration::Client(ClientConfiguration {
        ssid,
        password,
        auth_method,
        ..Default::default()
    });

    wifi.set_configuration(&cfg)?;
    ensure_connected(&mut wifi)?;

    Ok(wifi)
}

pub fn ensure_connected(wifi: &mut BlockingWifi<EspWifi<'static>>) -> Result<()> {
    if !wifi.is_started()? {
        wifi.start()?;
    }
    if !wifi.is_connected()? {
        wifi.connect()?;
    }
    wifi.wait_netif_up()?;
    Ok(())
}
