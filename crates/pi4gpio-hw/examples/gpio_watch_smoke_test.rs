//! Manual GPIO edge-capture smoke test using a generated toggle pattern.

use pi4gpio_hw::gpio::{GpioChip, Level, PullMode};
use pi4gpio_hw::gpio_watch::{EdgeEvent, EdgeWatcher};
use std::process::ExitCode;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const CHIP_PATH: &str = "/dev/gpiochip0";
const TOGGLE_INTERVAL: Duration = Duration::from_millis(20);
const TOGGLE_COUNT: usize = 6;

fn main() -> ExitCode {
    let pin: u32 = match std::env::args().nth(1).and_then(|s| s.parse().ok()) {
        Some(pin) => pin,
        None => {
            eprintln!("usage: gpio_watch_smoke_test <pin>");
            return ExitCode::FAILURE;
        }
    };

    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let watcher_thread = thread::spawn(move || -> Result<Vec<EdgeEvent>, String> {
        let mut watcher =
            EdgeWatcher::open(CHIP_PATH, pin, PullMode::None).map_err(|e| e.to_string())?;
        ready_tx.send(()).ok();
        watcher
            .wait_events(Duration::from_secs(3), TOGGLE_COUNT)
            .map_err(|e| e.to_string())
    });

    if ready_rx.recv_timeout(Duration::from_secs(2)).is_err() {
        eprintln!("timed out waiting for EdgeWatcher to start");
        return ExitCode::FAILURE;
    }
    thread::sleep(Duration::from_millis(200));

    let mut chip = match GpioChip::open() {
        Ok(chip) => chip,
        Err(err) => {
            eprintln!("GpioChip::open failed: {err}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(err) = chip.claim_output(pin) {
        eprintln!("claim_output failed: {err}");
        return ExitCode::FAILURE;
    }

    println!("generating a known toggle pattern (pin={pin}, count={TOGGLE_COUNT}, interval={TOGGLE_INTERVAL:?})");
    let mut level = Level::Low;
    for _ in 0..TOGGLE_COUNT {
        level = if level == Level::Low {
            Level::High
        } else {
            Level::Low
        };
        if let Err(err) = chip.write(pin, level) {
            eprintln!("write failed: {err}");
            return ExitCode::FAILURE;
        }
        thread::sleep(TOGGLE_INTERVAL);
    }

    let events = match watcher_thread.join() {
        Ok(Ok(events)) => events,
        Ok(Err(err)) => {
            eprintln!("EdgeWatcher failed: {err}");
            return ExitCode::FAILURE;
        }
        Err(_) => {
            eprintln!("watcher thread panicked");
            return ExitCode::FAILURE;
        }
    };

    println!(
        "recorded edges: {} (expected: {TOGGLE_COUNT})",
        events.len()
    );
    let mut prev_ts: Option<u64> = None;
    for (i, e) in events.iter().enumerate() {
        let delta_ms = prev_ts.map(|p| (e.timestamp_ns - p) as f64 / 1_000_000.0);
        let delta_str = delta_ms
            .map(|d| format!(" ({d:.2} ms since previous)"))
            .unwrap_or_default();
        let dir = if e.rising { "rising" } else { "falling" };
        println!("  [{i}] {dir} @ {}ns{delta_str}", e.timestamp_ns);
        prev_ts = Some(e.timestamp_ns);
    }

    if events.len() == TOGGLE_COUNT {
        println!("all checks passed");
        ExitCode::SUCCESS
    } else {
        println!("edge count did not match the expected value");
        ExitCode::FAILURE
    }
}
