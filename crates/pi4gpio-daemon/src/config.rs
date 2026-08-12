//! 設定読み込み。
//!
//! 雛形段階では環境変数のみに対応。
//! TODO: 設定ファイルからの読み込み、Tailscale限定bindオプション、
//! APIキー設定（NETWORK_POLICY.md）を実装する。

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
