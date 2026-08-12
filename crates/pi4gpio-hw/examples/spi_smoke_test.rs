//! Manual SPI transfer smoke test using an MCP3208-compatible command.

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
