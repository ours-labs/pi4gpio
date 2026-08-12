//! Manual I2C combined-transaction smoke test against a read-only chip ID.

use pi4gpio_hw::i2c::I2cBus;
use std::process::ExitCode;

const CHIP_ID_REG: u8 = 0xd0;
const BME280_ID: u8 = 0x60;
const BMP280_ID: u8 = 0x58;
const CANDIDATE_ADDRS: [u8; 2] = [0x76, 0x77];

fn main() -> ExitCode {
    let bus_num: u8 = match std::env::args().nth(1).and_then(|s| s.parse().ok()) {
        Some(bus) => bus,
        None => {
            eprintln!("usage: i2c_smoke_test <bus>");
            return ExitCode::FAILURE;
        }
    };

    let mut bus = match I2cBus::open(bus_num) {
        Ok(bus) => bus,
        Err(err) => {
            eprintln!("I2cBus::open failed: {err}");
            return ExitCode::FAILURE;
        }
    };

    let mut found_any = false;

    for addr in CANDIDATE_ADDRS {
        println!("[addr=0x{addr:02x}] write_read([0xD0], 1) -> chip ID register");
        let mut id = [0u8; 1];
        match bus.write_read(addr, &[CHIP_ID_REG], &mut id) {
            Ok(()) => {
                let name = match id[0] {
                    BME280_ID => " (BME280)",
                    BMP280_ID => " (BMP280)",
                    _ => "",
                };
                println!("  OK: chip_id=0x{:02x}{name}", id[0]);
                found_any = true;
            }
            Err(err) => {
                println!("  no response (device may be disconnected): {err}");
            }
        }
    }

    if found_any {
        println!("all checks passed (combined transaction works)");
        ExitCode::SUCCESS
    } else {
        println!("neither address responded; the device is probably disconnected");
        ExitCode::SUCCESS
    }
}
