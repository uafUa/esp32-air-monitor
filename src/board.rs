use anyhow::Result;
use esp_idf_hal::gpio::AnyIOPin;
use esp_idf_hal::i2c::I2cDriver;
use esp_idf_hal::ledc;
use esp_idf_hal::peripherals::Peripherals;

use crate::battery::Battery;
use crate::st7789::{init_lcd, St7789};
use crate::mhz19b::{init_mhz19b, Mhz19b};
use crate::sht31::Sht31;
use crate::touch::init_i2c;
use crate::wifi::init_wifi;
use log::warn;

pub struct Board {
    pub lcd: St7789<'static, ledc::TIMER0>,
    pub i2c: I2cDriver<'static>,
    pub mhz19b: Mhz19b<'static>,
    pub battery: Battery<'static>,
    pub sht31: Sht31,
    pub wifi: Option<esp_idf_svc::wifi::BlockingWifi<esp_idf_svc::wifi::EspWifi<'static>>>,
}

impl Board {
    pub fn init() -> Result<Self> {
        let Peripherals {
            pins,
            i2c0,
            uart0,
            spi2,
            ledc,
            adc1,
            modem,
            ..
        } = Peripherals::take()?;

        let i2c = init_i2c(i2c0, pins.gpio18, pins.gpio19)?;
        let mut mhz19b = init_mhz19b(uart0, pins.gpio16, pins.gpio17)?;
        mhz19b.set_abc(false)?;
        let sht31 = Sht31::new_default();
        let wifi = match init_wifi(modem) {
            Ok(wifi) => Some(wifi),
            Err(err) => {
                warn!("Wi-Fi init failed: {:?}", err);
                None
            }
        };
        let lcd = init_lcd(
            spi2,
            ledc,
            AnyIOPin::from(pins.gpio1),
            AnyIOPin::from(pins.gpio2),
            AnyIOPin::from(pins.gpio14),
            AnyIOPin::from(pins.gpio15),
            AnyIOPin::from(pins.gpio22),
            AnyIOPin::from(pins.gpio23),
        )?;
        let battery = Battery::new(adc1, pins.gpio0)?;

        Ok(Self {
            lcd,
            i2c,
            mhz19b,
            battery,
            sht31,
            wifi,
        })
    }
}
