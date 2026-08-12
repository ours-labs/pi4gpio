//! `pi4gpio-hw`のSPI実装を実機で手動検証するためのサンプル。
//! CIでは実行できない（実ハードウェアが必要）。
//!
//! Sends an MCP3208 channel-read command (`[cmd1, cmd2, 0]`) and prints
//! 同じ`[cmd1, cmd2, 0]`）を送り、受信バイト列を表示する。SPIはI2Cと
//! 違いACK/NACKが無いため「応答がある/ない」で正誤判定はできない。
//! センサー基盤が物理的に未接続の場合と、実際に正しく動いている場合を
//! 区別するには、同じコマンドをPython(spidev)側でも実行し受信バイト列を
//! 突き合わせるのが確実——この実行結果は別途Python側と比較する。
//!
//! 使い方: cargo run --release --example spi_smoke_test -- <bus> <chip_select> <channel>

use pi4gpio_hw::spi::SpiDevice;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (Some(bus), Some(cs), Some(channel)) = (
        args.next().and_then(|s| s.parse::<u8>().ok()),
        args.next().and_then(|s| s.parse::<u8>().ok()),
        args.next().and_then(|s| s.parse::<u8>().ok()),
    ) else {
        eprintln!("usage: spi_smoke_test <bus> <chip_select> <channel>");
        return ExitCode::FAILURE;
    };

    let mut device = match SpiDevice::open(bus, cs) {
        Ok(device) => device,
        Err(err) => {
            eprintln!("SpiDevice::open failed: {err}");
            return ExitCode::FAILURE;
        }
    };

    // MCP3208 single-channel read command.
    let cmd1 = 0x06 | (channel >> 2);
    let cmd2 = (channel & 3) << 6;
    let tx = [cmd1, cmd2, 0];
    let mut rx = [0u8; 3];

    match device.transfer(&tx, &mut rx) {
        Ok(()) => {
            let value = ((rx[1] as u16 & 0x0f) << 8) | rx[2] as u16;
            println!("tx={tx:02x?} rx={rx:02x?} value={value} (0-4095)");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("transfer failed: {err}");
            ExitCode::FAILURE
        }
    }
}
