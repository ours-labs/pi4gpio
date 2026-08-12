//! Hardware access primitives for Raspberry Pi 4 GPIO, I2C, SPI, and UART.
#![deny(unsafe_op_in_unsafe_fn)]

pub mod error;
pub mod gpio;
pub mod gpio_watch;
pub mod i2c;
pub mod spi;
pub mod uart;

pub use error::HwError;
