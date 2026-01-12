use anyhow::Result;
use esp_idf_hal::adc::attenuation::DB_11;
use esp_idf_hal::adc::oneshot::config::{AdcChannelConfig, Calibration};
use esp_idf_hal::adc::oneshot::{AdcChannelDriver, AdcDriver};
use esp_idf_hal::adc::ADC1;
use esp_idf_hal::gpio::Gpio0;
use esp_idf_hal::peripheral::Peripheral;

const BATTERY_SCALE: f32 = 3.0;

pub struct Battery<'d> {
    channel: AdcChannelDriver<'d, Gpio0, AdcDriver<'d, ADC1>>,
}

impl<'d> Battery<'d> {
    pub fn new(
        adc: impl Peripheral<P = ADC1> + 'd,
        pin: impl Peripheral<P = Gpio0> + 'd,
    ) -> Result<Self> {
        let adc = AdcDriver::new(adc)?;
        let config = AdcChannelConfig {
            attenuation: DB_11,
            calibration: calibration_mode(),
            ..Default::default()
        };
        let channel = AdcChannelDriver::new(adc, pin, &config)?;
        Ok(Self { channel })
    }

    pub fn read_voltage(&mut self) -> Result<f32> {
        let mv = self.channel.read()? as f32;
        Ok((mv / 1000.0) * BATTERY_SCALE)
    }
}

#[cfg(all(
    any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled),
    any(
        esp32c3,
        all(
            esp32c6,
            not(all(esp_idf_version_major = "5", esp_idf_version_minor = "0")),
            not(esp_idf_version_full = "5.1.0")
        ),
        esp32s3
    )
))]
fn calibration_mode() -> Calibration {
    Calibration::Curve
}

#[cfg(not(all(
    any(esp_idf_comp_esp_adc_cal_enabled, esp_idf_comp_esp_adc_enabled),
    any(
        esp32c3,
        all(
            esp32c6,
            not(all(esp_idf_version_major = "5", esp_idf_version_minor = "0")),
            not(esp_idf_version_full = "5.1.0")
        ),
        esp32s3
    )
)))]
fn calibration_mode() -> Calibration {
    Calibration::None
}
