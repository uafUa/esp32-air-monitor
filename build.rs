use std::fs;
use std::path::PathBuf;

fn main() {
    // Propagate ESP-IDF link/cfg/include args from esp-idf-sys.
    if let Err(err) = embuild::build::LinkArgs::output_propagated("ESP_IDF") {
        println!("cargo:warning=esp-idf link args not propagated: {err}");
    }
    if let Err(err) = embuild::build::CfgArgs::output_propagated("ESP_IDF") {
        println!("cargo:warning=esp-idf cfg args not propagated: {err}");
    }

    println!("cargo:rerun-if-env-changed=OTA_BUILD");
    println!("cargo:rerun-if-env-changed=OTA_BASE_URL");
    println!("cargo:rerun-if-env-changed=WIFI_SSID");
    println!("cargo:rerun-if-env-changed=WIFI_PASS");
    println!("cargo:rerun-if-env-changed=MQTT_HOST");
    println!("cargo:rerun-if-env-changed=MQTT_PORT");
    println!("cargo:rerun-if-env-changed=MQTT_USER");
    println!("cargo:rerun-if-env-changed=MQTT_PASS");
    println!("cargo:rerun-if-env-changed=MQTT_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=MQTT_PREFIX");

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let build_file = manifest_dir.join("scripts").join("build-number.txt");
    println!("cargo:rerun-if-changed={}", build_file.display());

    let mut build = std::env::var("OTA_BUILD").ok().filter(|v| !v.trim().is_empty());
    if build.is_none() {
        if let Ok(contents) = fs::read_to_string(&build_file) {
            let candidate = contents.trim().to_string();
            if !candidate.is_empty() {
                build = Some(candidate);
            }
        }
    }

    if let Some(value) = build {
        println!("cargo:rustc-env=OTA_BUILD={}", value);
    }
}
