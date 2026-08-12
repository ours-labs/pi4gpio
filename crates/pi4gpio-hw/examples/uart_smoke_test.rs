//! `pi4gpio-hw`のUART実装を実機で手動検証するためのサンプル。
//! CIでは実行できない（実ハードウェアが必要）。
//!
//! Sends the 9-byte MH-Z19C read command and receives its response.
//! 9バイトの応答を待つ。呼び出し側でUARTの排他アクセスを確保してから
//! 実行することを前提にしている——termios設定はプロセス単位
//! ではなくデバイス単位の共有状態のため、本番プロセスと同時に触ると
//! コマンド/レスポンスが混線しうる（I2C/SPIとは異なりカーネルが
//! トランザクション単位で守ってくれない）。
//!
//! 使い方: cargo run --release --example uart_smoke_test -- <device> <baud>

use pi4gpio_hw::uart::UartPort;
use std::process::ExitCode;

/// MH-Z19C CO2 read command from the device protocol.
const READ_CO2_COMMAND: [u8; 9] = [0xff, 0x01, 0x86, 0x00, 0x00, 0x00, 0x00, 0x00, 0x79];

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (Some(device), Some(baud)) = (args.next(), args.next().and_then(|s| s.parse::<u32>().ok()))
    else {
        eprintln!("usage: uart_smoke_test <device> <baud>");
        return ExitCode::FAILURE;
    };

    let mut port = match UartPort::open(&device, baud) {
        Ok(port) => port,
        Err(err) => {
            eprintln!("UartPort::open failed: {err}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(err) = port.write(&READ_CO2_COMMAND) {
        eprintln!("write failed: {err}");
        return ExitCode::FAILURE;
    }

    // Allow the MH-Z19C time to produce a response.
    std::thread::sleep(std::time::Duration::from_millis(100));

    let mut buf = [0u8; 9];
    match port.read(&mut buf) {
        Ok(n) => {
            println!("received {n} bytes: {:02x?}", &buf[..n]);
            if n == 9 && buf[0] == 0xff && buf[1] == 0x86 {
                let co2_ppm = (buf[2] as u16) * 256 + buf[3] as u16;
                println!("CO2濃度: {co2_ppm} ppm（応答形式は正常）");
            } else {
                println!("応答なし、または想定形式と不一致（センサー未接続の可能性）");
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("read failed: {err}");
            ExitCode::FAILURE
        }
    }
}
