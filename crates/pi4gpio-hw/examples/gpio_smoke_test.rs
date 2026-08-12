//! `pi4gpio-hw`のGPIO実装を実機で手動検証するためのサンプル。
//! CIでは実行できない（実ハードウェアが必要）。
//!
//! 使い方: cargo run --release --example gpio_smoke_test -- <pin>
//! 対象ピンは他の何にも使われていない未接続ピンであること
//! （`gpioinfo`で consumer が無いことを事前に確認する）。

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

    println!("[pull-up] claim_input(pin={pin}, Up) -> 未接続なら High のはず");
    let result = chip
        .claim_input(pin, PullMode::Up)
        .and_then(|()| chip.read(pin));
    check(&mut failures, result, Level::High);

    println!("[pull-down] claim_input(pin={pin}, Down) -> 未接続なら Low のはず");
    let result = chip
        .claim_input(pin, PullMode::Down)
        .and_then(|()| chip.read(pin));
    check(&mut failures, result, Level::Low);

    println!("[output-high] claim_output + write(High) -> read で High が読み戻るはず");
    let result = chip
        .claim_output(pin)
        .and_then(|()| chip.write(pin, Level::High))
        .and_then(|()| chip.read(pin));
    check(&mut failures, result, Level::High);

    println!("[output-low] write(Low) -> read で Low が読み戻るはず");
    let result = chip.write(pin, Level::Low).and_then(|()| chip.read(pin));
    check(&mut failures, result, Level::Low);

    // 後始末: 入力・プルなしに戻しておく。
    let _ = chip.claim_input(pin, PullMode::None);

    if failures == 0 {
        println!("すべて成功");
        ExitCode::SUCCESS
    } else {
        println!("{failures}件失敗");
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
