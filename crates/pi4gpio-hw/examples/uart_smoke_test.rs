//! Manual UART smoke test using an MH-Z19C-compatible request frame.

use pi4gpio_hw::uart::UartPort;
use std::process::ExitCode;

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

    std::thread::sleep(std::time::Duration::from_millis(100));

    let mut buf = [0u8; 9];
    match port.read(&mut buf) {
        Ok(n) => {
            println!("received {n} bytes: {:02x?}", &buf[..n]);
            if n == 9 && buf[0] == 0xff && buf[1] == 0x86 {
                let co2_ppm = (buf[2] as u16) * 256 + buf[3] as u16;
                println!("CO2 concentration: {co2_ppm} ppm (valid response format)");
            } else {
                println!("no response or unexpected format (sensor may be disconnected)");
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("read failed: {err}");
            ExitCode::FAILURE
        }
    }
}
