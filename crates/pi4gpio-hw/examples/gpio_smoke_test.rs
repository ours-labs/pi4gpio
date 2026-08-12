//! Manual GPIO read/write smoke test for an otherwise unused pin.

use pi4gpio_hw::gpio::{GpioChip, Level, PullMode};
use pi4gpio_hw::HwError;
use std::process::ExitCode;

fn main() -> ExitCode {
    let pin: u32 = match std::env::args().nth(1).and_then(|s| s.parse().ok()) {
        Some(pin) => pin,
        None => {
            eprintln!("usage: gpio_smoke_test <pin>");
            return ExitCode::FAILURE;
        }
    };

    let mut chip = match GpioChip::open() {
        Ok(chip) => chip,
        Err(err) => {
            eprintln!("GpioChip::open failed: {err}");
            return ExitCode::FAILURE;
        }
    };

    let mut failures = 0u32;

    println!("[pull-up] claim_input(pin={pin}, Up) -> expected High when unconnected");
    let result = chip
        .claim_input(pin, PullMode::Up)
        .and_then(|()| chip.read(pin));
    check(&mut failures, result, Level::High);

    println!("[pull-down] claim_input(pin={pin}, Down) -> expected Low when unconnected");
    let result = chip
        .claim_input(pin, PullMode::Down)
        .and_then(|()| chip.read(pin));
    check(&mut failures, result, Level::Low);

    println!("[output-high] claim_output + write(High) -> read should return High");
    let result = chip
        .claim_output(pin)
        .and_then(|()| chip.write(pin, Level::High))
        .and_then(|()| chip.read(pin));
    check(&mut failures, result, Level::High);

    println!("[output-low] write(Low) -> read should return Low");
    let result = chip.write(pin, Level::Low).and_then(|()| chip.read(pin));
    check(&mut failures, result, Level::Low);

    let _ = chip.claim_input(pin, PullMode::None);

    if failures == 0 {
        println!("all checks passed");
        ExitCode::SUCCESS
    } else {
        println!("{failures} checks failed");
        ExitCode::FAILURE
    }
}

fn check(failures: &mut u32, result: Result<Level, HwError>, expected: Level) {
    match result {
        Ok(level) if level == expected => println!("  OK ({level:?})"),
        Ok(level) => {
            println!("  NG: got {level:?}, expected {expected:?}");
            *failures += 1;
        }
        Err(err) => {
            println!("  ERROR: {err}");
            *failures += 1;
        }
    }
}
