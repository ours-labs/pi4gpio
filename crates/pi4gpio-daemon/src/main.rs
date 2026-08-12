mod client;
mod config;
mod lock;
mod protocol;
mod socket;

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let config = config::Config::load();

    println!("pi4gpiod starting (socket: {})", config.socket_path);

    if let Err(err) = socket::serve(&config).await {
        eprintln!("pi4gpiod: fatal error: {err}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
