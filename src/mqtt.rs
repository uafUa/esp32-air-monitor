use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
// embedded-svc defines the MQTT traits and event payloads used across platforms.
use embedded_svc::mqtt::client::{EventPayload, QoS};
// esp-idf-svc provides the ESP-IDF backed MQTT client implementation and config.
use esp_idf_svc::mqtt::client::{EspMqttClient, LwtConfiguration, MqttClientConfiguration};
use log::{info, warn};

use crate::wifi::ensure_connected;

const MQTT_HOST: &str = match option_env!("MQTT_HOST") {
    Some(v) => v,
    None => "homeassistant.local",
};
const MQTT_PORT_DEFAULT: u16 = 1883;
const MQTT_PORT_ENV: Option<&str> = option_env!("MQTT_PORT");
const MQTT_USER: Option<&str> = match option_env!("MQTT_USER") {
    Some(v) if !v.is_empty() => Some(v),
    _ => Some("esp32-co"),
};
const MQTT_PASS: Option<&str> = match option_env!("MQTT_PASS") {
    Some(v) if !v.is_empty() => Some(v),
    _ => Some("esp32-co"),
};
const MQTT_CLIENT_ID: &str = match option_env!("MQTT_CLIENT_ID") {
    Some(v) => v,
    None => "c6-demo",
};
const MQTT_PREFIX: &str = match option_env!("MQTT_PREFIX") {
    Some(v) => v,
    None => "c6-demo",
};
const OTA_BUILD: Option<&str> = option_env!("OTA_BUILD");
const SW_VERSION: &str = env!("CARGO_PKG_VERSION");

const PAYLOAD_ONLINE: &str = "online";
const PAYLOAD_OFFLINE: &str = "offline";

#[derive(Debug)]
pub enum Command {
    ZeroCalibrate,
    SetAbc(bool),
    SetBrightness(u8),
    Reboot,
}

#[derive(Default, Debug, Clone)]
pub struct Telemetry {
    pub co2_ppm: Option<u16>,
    pub temp_c: Option<f32>,
    pub humidity_pct: Option<u8>,
    pub battery_v: Option<f32>,
}

struct Topics {
    availability: String,
    status: String,
    cmd: String,
}

pub struct MqttClient {
    client: EspMqttClient<'static>,
    cmd_rx: Receiver<Command>,
    topics: Topics,
}

impl MqttClient {
    pub fn publish_status(&mut self, telemetry: &Telemetry) -> Result<()> {
        let payload = telemetry_payload(telemetry);
        self.client
            .publish(&self.topics.status, QoS::AtMostOnce, false, payload.as_bytes())?;
        Ok(())
    }

    pub fn try_recv_command(&mut self) -> Option<Command> {
        self.cmd_rx.try_recv().ok()
    }
}

pub fn init_mqtt(
    wifi: &mut esp_idf_svc::wifi::BlockingWifi<esp_idf_svc::wifi::EspWifi<'static>>,
) -> Result<MqttClient> {
    // Ensure Wi-Fi is connected before starting the MQTT client.
    ensure_connected(wifi)?;

    let topics = Topics {
        availability: format!("{}/availability", MQTT_PREFIX),
        status: format!("{}/status", MQTT_PREFIX),
        cmd: format!("{}/cmd", MQTT_PREFIX),
    };

    let port = MQTT_PORT_ENV
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(MQTT_PORT_DEFAULT);
    let url = format!("mqtt://{}:{}", MQTT_HOST, port);
    // ESP-IDF MQTT client configuration (LWT, auth, keepalive, timeouts).
    let mut conf = MqttClientConfiguration::default();
    conf.client_id = Some(MQTT_CLIENT_ID);
    conf.username = MQTT_USER;
    conf.password = MQTT_PASS;
    conf.keep_alive_interval = Some(Duration::from_secs(30));
    conf.network_timeout = Duration::from_secs(5);
    conf.lwt = Some(LwtConfiguration {
        topic: &topics.availability,
        payload: PAYLOAD_OFFLINE.as_bytes(),
        qos: QoS::AtLeastOnce,
        retain: true,
    });

    // Create the client plus a connection event iterator.
    let (mut client, mut conn) = EspMqttClient::new(&url, &conf)?;
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    let (conn_tx, conn_rx) = mpsc::channel::<bool>();
    let cmd_topic = topics.cmd.clone();

    // Event loop runs on a separate thread; it receives MQTT events from ESP-IDF.
    thread::spawn(move || loop {
        match conn.next() {
            Ok(event) => {
                match event.payload() {
                    EventPayload::Connected(_) => {
                        let _ = conn_tx.send(true);
                    }
                    EventPayload::Disconnected => {
                        let _ = conn_tx.send(false);
                    }
                    EventPayload::Received { topic, data, .. } => {
                        if topic == Some(cmd_topic.as_str()) {
                            if let Some(command) = parse_command(data) {
                                let _ = cmd_tx.send(command);
                            } else {
                                warn!("MQTT command ignored: {:?}", String::from_utf8_lossy(data));
                            }
                        }
                    }
                    _ => {}
                }
            }
            Err(_) => {
                thread::sleep(Duration::from_millis(100));
            }
        }
    });

    // Wait for the broker connection before we publish/subscribe.
    match conn_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(true) => {}
        Ok(false) => return Err(anyhow!("MQTT disconnected during init")),
        Err(_) => return Err(anyhow!("MQTT connect timeout")),
    }

    // Subscribe for commands after connection succeeds.
    client.subscribe(&topics.cmd, QoS::AtLeastOnce)?;
    client.publish(
        &topics.availability,
        QoS::AtLeastOnce,
        true,
        PAYLOAD_ONLINE.as_bytes(),
    )?;
    // Publish HomeAssistant discovery configs so entities show up automatically.
    publish_discovery(&mut client, &topics)?;

    info!("MQTT connected to {}", url);
    Ok(MqttClient {
        client,
        cmd_rx,
        topics,
    })
}

fn parse_command(payload: &[u8]) -> Option<Command> {
    let text = String::from_utf8_lossy(payload);
    let text = text.trim().to_ascii_lowercase();
    if text.is_empty() {
        return None;
    }
    if text == "zero" || text == "zero_calibrate" {
        return Some(Command::ZeroCalibrate);
    }
    if text == "reboot" {
        return Some(Command::Reboot);
    }
    if let Some(value) = text.strip_prefix("abc=") {
        return parse_on_off(value).map(Command::SetAbc);
    }
    if let Some(value) = text.strip_prefix("abc:") {
        return parse_on_off(value).map(Command::SetAbc);
    }
    if let Some(value) = text.strip_prefix("brightness=") {
        return parse_percent(value).map(Command::SetBrightness);
    }
    if let Some(value) = text.strip_prefix("brightness:") {
        return parse_percent(value).map(Command::SetBrightness);
    }
    None
}

fn parse_on_off(value: &str) -> Option<bool> {
    match value.trim() {
        "1" | "on" | "true" => Some(true),
        "0" | "off" | "false" => Some(false),
        _ => None,
    }
}

fn parse_percent(value: &str) -> Option<u8> {
    let raw = value.trim().parse::<u8>().ok()?;
    Some(raw.min(100))
}

fn telemetry_payload(t: &Telemetry) -> String {
    let co2 = t
        .co2_ppm
        .map(|v| v.to_string())
        .unwrap_or_else(|| "null".to_string());
    let temp = t
        .temp_c
        .map(|v| format!("{:.1}", v))
        .unwrap_or_else(|| "null".to_string());
    let hum = t
        .humidity_pct
        .map(|v| v.to_string())
        .unwrap_or_else(|| "null".to_string());
    let bat = t
        .battery_v
        .map(|v| format!("{:.2}", v))
        .unwrap_or_else(|| "null".to_string());

    format!(
        r#"{{"co2_ppm":{co2},"temp_c":{temp},"humidity_pct":{hum},"battery_v":{bat}}}"#
    )
}

fn publish_discovery(client: &mut EspMqttClient<'static>, topics: &Topics) -> Result<()> {
    let device_id = MQTT_PREFIX.replace('-', "_");
    let device_name = "C6 Demo";
    let sw_version = match OTA_BUILD {
        Some(build) => format!("{SW_VERSION}+{build}"),
        None => SW_VERSION.to_string(),
    };

    let device = format!(
        r#""device":{{"identifiers":["{device_id}"],"name":"{device_name}","model":"ESP32-C6 Touch LCD 1.47","manufacturer":"Espressif","sw_version":"{sw_version}"}}"#
    );

    // CO2 sensor entity: uses value_template to pull co2_ppm from the JSON status payload.
    publish_sensor_config(
        client,
        &device_id,
        "co2",
        "C6 CO2",
        topics,
        r#"{{ value_json.co2_ppm }}"#,
        Some("ppm"),
        Some("carbon_dioxide"),
        Some("measurement"),
        &device,
    )?;
    // Temperature sensor entity (°C) from JSON status payload.
    publish_sensor_config(
        client,
        &device_id,
        "temperature",
        "C6 Temperature",
        topics,
        r#"{{ value_json.temp_c }}"#,
        Some("°C"),
        Some("temperature"),
        Some("measurement"),
        &device,
    )?;
    // Humidity sensor entity (%) from JSON status payload.
    publish_sensor_config(
        client,
        &device_id,
        "humidity",
        "C6 Humidity",
        topics,
        r#"{{ value_json.humidity_pct }}"#,
        Some("%"),
        Some("humidity"),
        Some("measurement"),
        &device,
    )?;
    // Battery voltage sensor entity (V) from JSON status payload.
    publish_sensor_config(
        client,
        &device_id,
        "battery",
        "C6 Battery",
        topics,
        r#"{{ value_json.battery_v }}"#,
        Some("V"),
        Some("voltage"),
        Some("measurement"),
        &device,
    )?;
    // Button entity: publishes "zero_calibrate" to <prefix>/cmd when pressed.
    publish_button_config(
        client,
        &device_id,
        "zero_calibrate",
        "C6 Zero Calibrate",
        topics,
        "zero_calibrate",
        &device,
    )?;
    // Button entity: publishes "reboot" to <prefix>/cmd when pressed.
    publish_button_config(
        client,
        &device_id,
        "reboot",
        "C6 Reboot",
        topics,
        "reboot",
        &device,
    )?;
    // Switch entity (optimistic): publishes "abc:on"/"abc:off" to <prefix>/cmd.
    publish_switch_config(
        client,
        &device_id,
        "abc",
        "C6 ABC",
        topics,
        "abc:on",
        "abc:off",
        &device,
    )?;
    // Number entity (optimistic slider 0..100): publishes "brightness:<value>" to <prefix>/cmd.
    publish_number_config(
        client,
        &device_id,
        "brightness",
        "C6 Brightness",
        topics,
        0,
        100,
        1,
        &device,
    )?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn publish_sensor_config(
    client: &mut EspMqttClient<'static>,
    device_id: &str,
    key: &str,
    name: &str,
    topics: &Topics,
    value_template: &str,
    unit: Option<&str>,
    device_class: Option<&str>,
    state_class: Option<&str>,
    device: &str,
) -> Result<()> {
    // HomeAssistant MQTT sensor discovery payload.
    let mut payload = format!(
        r#"{{"name":"{name}","state_topic":"{state_topic}","value_template":"{value_template}","availability_topic":"{availability_topic}","payload_available":"{online}","payload_not_available":"{offline}","unique_id":"{device_id}-{key}","#,
        state_topic = topics.status,
        availability_topic = topics.availability,
        online = PAYLOAD_ONLINE,
        offline = PAYLOAD_OFFLINE,
    );

    if let Some(unit) = unit {
        payload.push_str(&format!(r#""unit_of_measurement":"{unit}","#));
    }
    if let Some(device_class) = device_class {
        payload.push_str(&format!(r#""device_class":"{device_class}","#));
    }
    if let Some(state_class) = state_class {
        payload.push_str(&format!(r#""state_class":"{state_class}","#));
    }
    payload.push_str(device);
    payload.push('}');

    let topic = format!("homeassistant/sensor/{device_id}/{key}/config");
    client.publish(&topic, QoS::AtLeastOnce, true, payload.as_bytes())?;
    Ok(())
}

fn publish_button_config(
    client: &mut EspMqttClient<'static>,
    device_id: &str,
    key: &str,
    name: &str,
    topics: &Topics,
    payload_press: &str,
    device: &str,
) -> Result<()> {
    // HomeAssistant MQTT button discovery payload (stateless action).
    let payload = format!(
        r#"{{"name":"{name}","command_topic":"{command_topic}","payload_press":"{payload_press}","availability_topic":"{availability_topic}","payload_available":"{online}","payload_not_available":"{offline}","unique_id":"{device_id}-{key}",{device}}}"#,
        command_topic = topics.cmd,
        availability_topic = topics.availability,
        online = PAYLOAD_ONLINE,
        offline = PAYLOAD_OFFLINE,
    );

    let topic = format!("homeassistant/button/{device_id}/{key}/config");
    client.publish(&topic, QoS::AtLeastOnce, true, payload.as_bytes())?;
    Ok(())
}

fn publish_switch_config(
    client: &mut EspMqttClient<'static>,
    device_id: &str,
    key: &str,
    name: &str,
    topics: &Topics,
    payload_on: &str,
    payload_off: &str,
    device: &str,
) -> Result<()> {
    // HomeAssistant MQTT switch discovery payload (optimistic, no state topic).
    let payload = format!(
        r#"{{"name":"{name}","command_topic":"{command_topic}","payload_on":"{payload_on}","payload_off":"{payload_off}","optimistic":true,"availability_topic":"{availability_topic}","payload_available":"{online}","payload_not_available":"{offline}","unique_id":"{device_id}-{key}",{device}}}"#,
        command_topic = topics.cmd,
        availability_topic = topics.availability,
        online = PAYLOAD_ONLINE,
        offline = PAYLOAD_OFFLINE,
    );

    let topic = format!("homeassistant/switch/{device_id}/{key}/config");
    client.publish(&topic, QoS::AtLeastOnce, true, payload.as_bytes())?;
    Ok(())
}

fn publish_number_config(
    client: &mut EspMqttClient<'static>,
    device_id: &str,
    key: &str,
    name: &str,
    topics: &Topics,
    min: i32,
    max: i32,
    step: i32,
    device: &str,
) -> Result<()> {
    // HomeAssistant MQTT number discovery payload (optimistic slider).
    let payload = format!(
        r#"{{"name":"{name}","command_topic":"{command_topic}","command_template":"brightness:{{{{ value }}}}","min":{min},"max":{max},"step":{step},"mode":"slider","unit_of_measurement":"%","optimistic":true,"availability_topic":"{availability_topic}","payload_available":"{online}","payload_not_available":"{offline}","unique_id":"{device_id}-{key}",{device}}}"#,
        command_topic = topics.cmd,
        availability_topic = topics.availability,
        online = PAYLOAD_ONLINE,
        offline = PAYLOAD_OFFLINE,
    );

    let topic = format!("homeassistant/number/{device_id}/{key}/config");
    client.publish(&topic, QoS::AtLeastOnce, true, payload.as_bytes())?;
    Ok(())
}
