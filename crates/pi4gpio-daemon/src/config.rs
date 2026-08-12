//! Local daemon configuration.
//!
//! The public transport is a Unix-domain socket. Network configuration is not
//! part of the current protocol or security boundary.

const DEFAULT_SOCKET_PATH: &str = "/run/pi4gpio/pi4gpio.sock";
const SOCKET_PATH_ENV: &str = "PI4GPIO_SOCKET_PATH";

pub struct Config {
    pub socket_path: String,
}

impl Config {
    pub fn load() -> Self {
        let socket_path =
            std::env::var(SOCKET_PATH_ENV).unwrap_or_else(|_| DEFAULT_SOCKET_PATH.to_string());
        Self { socket_path }
    }
}
