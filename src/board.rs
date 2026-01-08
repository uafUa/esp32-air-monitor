use anyhow::Result;
use esp_idf_hal::gpio::AnyIOPin;
use esp_idf_hal::i2c::I2cDriver;
use esp_idf_hal::ledc;
use esp_idf_hal::peripherals::Peripherals;

use crate::st7789::{init_lcd, St7789};
use crate::sensors::dht::Dht22Sensor;
use crate::sensors::mhz19b::Mhz19b;
use crate::sensors::{dht::init_dht22, mhz19b::init_mhz19b};
use crate::touch::init_i2c;

pub struct Board {
    pub lcd: St7789<'static, ledc::TIMER0>,
    pub i2c: I2cDriver<'static>,
    pub dht22: Dht22Sensor<'static, esp_idf_hal::gpio::Gpio4>,
    pub mhz19b: Mhz19b<'static>,
}

impl Board {
    pub fn init() -> Result<Self> {
        let Peripherals {
            pins,
            i2c0,
            uart0,
            spi2,
            ledc,
            ..
        } = Peripherals::take()?;

        let i2c = init_i2c(i2c0, pins.gpio18, pins.gpio19)?;
        let dht22 = init_dht22(pins.gpio4)?;
        let mhz19b = init_mhz19b(uart0, pins.gpio16, pins.gpio17)?;
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

        Ok(Self {
            lcd,
            i2c,
            dht22,
            mhz19b,
        })
    }
}
