//! Unixドメインソケットサーバ。
//!
//! NETWORK_POLICY.mdの決定に基づき、この段階ではローカルソケットのみを実装対象と
//! する。Tailscale限定bindは実際にリモート制御が必要になった時点で追加する。

use crate::client::ClientId;
use crate::config::Config;
use crate::lock::{BusId, LockTable};
use crate::protocol::{BusRef, EdgeEventWire, Operation, PullWire, Request, Response};
use pi4gpio_hw::gpio::{GpioChip, Level, PullMode};
use pi4gpio_hw::gpio_watch::{monotonic_now_ns, EdgeWatcher};
use pi4gpio_hw::i2c::I2cBus;
use pi4gpio_hw::spi::SpiDevice;
use pi4gpio_hw::uart::UartPort;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::io;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{signal, SignalKind};

const GPIOCHIP_PATH: &str = "/dev/gpiochip0";

/// I2Cバスはリクエストで指定された`bus`番号ごとに初回アクセス時に開く
/// （`/dev/i2c-0`と`/dev/i2c-1`の両方が存在しうるため、GPIOのようにプロセス
/// 起動時点で単一インスタンスを確保する構成にできない）。
type I2cBuses = HashMap<u8, I2cBus>;
/// SPIも同様に`(bus, chip_select)`ごとに初回アクセス時に開く。
type SpiDevices = HashMap<(u8, u8), SpiDevice>;
/// UARTも同様に`port`番号ごとに初回アクセス時に開く。`port`は
/// `/dev/ttyS{port}`に対応する（daemon側の命名規約）。
type UartPorts = HashMap<u8, UartPort>;

/// 遅延openしたデバイスハンドルのキャッシュ。値型をジェネリックにしているのは、
/// 実デバイスなしの単体テストでもdropを検証できるようにするため。
struct PeripheralHandles<I, S, U> {
    i2c: Mutex<HashMap<u8, I>>,
    spi: Mutex<HashMap<(u8, u8), S>>,
    uart: Mutex<HashMap<u8, U>>,
}

impl<I, S, U> Default for PeripheralHandles<I, S, U> {
    fn default() -> Self {
        Self {
            i2c: Mutex::new(HashMap::new()),
            spi: Mutex::new(HashMap::new()),
            uart: Mutex::new(HashMap::new()),
        }
    }
}

impl<I, S, U> PeripheralHandles<I, S, U> {
    /// 対応するキャッシュ要素をremoveし、その場でdropする。GPIOはdaemonの
    /// `/dev/gpiomem`マッピングを共有するため、ピン単位Releaseの対象外。
    fn close(&self, bus: BusId) -> bool {
        match bus {
            BusId::Gpio(_) => false,
            BusId::I2c(bus) => self
                .i2c
                .lock()
                .expect("i2c mutex poisoned")
                .remove(&bus)
                .is_some(),
            BusId::Spi(bus, chip_select) => self
                .spi
                .lock()
                .expect("spi mutex poisoned")
                .remove(&(bus, chip_select))
                .is_some(),
            BusId::Uart(port) => self
                .uart
                .lock()
                .expect("uart mutex poisoned")
                .remove(&port)
                .is_some(),
        }
    }
}

/// 各バス種別のハードウェア状態をまとめて保持する。ハンドラ関数の引数が
/// バス種別の数だけ増え続けるのを避けるための1つの塊として扱う。
struct Peripherals {
    locks: LockTable,
    gpio: Mutex<GpioChip>,
    handles: PeripheralHandles<I2cBus, SpiDevice, UartPort>,
}

fn remove_stale_socket(path: &Path) -> io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    if !metadata.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("refusing to replace non-socket path: {}", path.display()),
        ));
    }

    match StdUnixStream::connect(path) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("another pi4gpiod is listening on {}", path.display()),
        )),
        Err(err) if err.kind() == io::ErrorKind::ConnectionRefused => std::fs::remove_file(path),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

pub async fn serve(config: &Config) -> io::Result<()> {
    let socket_path = Path::new(&config.socket_path);
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    remove_stale_socket(socket_path)?;

    let listener = UnixListener::bind(socket_path)?;
    println!("pi4gpiod: listening on {}", config.socket_path);

    let peripherals = Arc::new(Peripherals {
        locks: LockTable::new(),
        gpio: Mutex::new(GpioChip::open().map_err(|e| io::Error::other(e.to_string()))?),
        handles: PeripheralHandles::default(),
    });
    let mut sigterm = signal(SignalKind::terminate())?;

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _addr) = accepted?;
                let peripherals = Arc::clone(&peripherals);
                tokio::spawn(async move {
                    if let Err(err) = handle_client(stream, peripherals).await {
                        eprintln!("pi4gpiod: client session ended with error: {err}");
                    }
                });
            }
            _ = sigterm.recv() => {
                println!("pi4gpiod: received SIGTERM, shutting down");
                break;
            }
            _ = tokio::signal::ctrl_c() => {
                println!("pi4gpiod: received SIGINT, shutting down");
                break;
            }
        }
    }

    let _ = std::fs::remove_file(&config.socket_path);
    Ok(())
}

async fn handle_client(stream: UnixStream, peripherals: Arc<Peripherals>) -> io::Result<()> {
    let client_id = ClientId::from_unix_stream(&stream)?;
    println!("pi4gpiod: client connected ({client_id:?})");

    let (reader, writer) = stream.into_split();
    let mut held_buses: HashSet<BusId> = HashSet::new();

    let result = process_requests(reader, writer, &client_id, &peripherals, &mut held_buses).await;

    // process_requestsがEOF（Ok）で終わってもI/Oエラー（Err、例えば
    // クライアントが強制切断されたことによるBroken pipe）で終わっても、
    // 保持中のロックは必ず解放する。以前は`?`によるアーリーリターンで
    // このブロック自体がスキップされることがあり、通信エラーで切断した
    // クライアントのロックが解放されないまま残ってしまうバグがあった
    // This also covers I/O-error exits so stale ownership cannot survive a disconnect.
    let buses: Vec<_> = held_buses.iter().copied().collect();
    for bus in buses {
        release_owned_bus(
            &peripherals.locks,
            &peripherals.handles,
            &client_id,
            &mut held_buses,
            bus,
        );
    }
    println!("pi4gpiod: client disconnected ({client_id:?})");
    result
}

async fn process_requests(
    reader: tokio::net::unix::OwnedReadHalf,
    mut writer: tokio::net::unix::OwnedWriteHalf,
    client_id: &ClientId,
    peripherals: &Arc<Peripherals>,
    held_buses: &mut HashSet<BusId>,
) -> io::Result<()> {
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => {
                // Tier 2のWatchEdgesは最大で数十ミリ秒ブロックしうる
                // （Tier 1の各操作はマイクロ秒オーダーで無視できる差だが、
                // これは無視できない）。tokioのワーカースレッドを塞がない
                // よう、実際のディスパッチはブロッキングスレッドプールで行う。
                let peripherals = Arc::clone(peripherals);
                let client_id = client_id.clone();
                let mut held = std::mem::take(held_buses);
                let (response, held) = tokio::task::spawn_blocking(move || {
                    let response = dispatch(&request, &client_id, &peripherals, &mut held);
                    (response, held)
                })
                .await
                .expect("dispatch task panicked");
                *held_buses = held;
                response
            }
            Err(err) => Response::malformed(&err.to_string()),
        };

        let mut payload =
            serde_json::to_vec(&response).expect("Response serialization cannot fail");
        payload.push(b'\n');
        writer.write_all(&payload).await?;
    }

    Ok(())
}

fn dispatch(
    request: &Request,
    client_id: &ClientId,
    peripherals: &Peripherals,
    held_buses: &mut HashSet<BusId>,
) -> Response {
    if let Err(error) = request.validate() {
        return Response::malformed(&error);
    }
    let bus: BusId = (&request.bus).into();

    if matches!(request.op, Operation::Release) {
        release_owned_bus(
            &peripherals.locks,
            &peripherals.handles,
            client_id,
            held_buses,
            bus,
        );
        return Response::ok();
    }

    if !held_buses.contains(&bus) {
        match peripherals.locks.try_acquire(bus, client_id.clone()) {
            Ok(()) => {
                held_buses.insert(bus);
            }
            Err(holder) => return Response::locked_by(&format!("{holder:?}")),
        }
    }

    match &request.bus {
        BusRef::Gpio { pin } => handle_gpio(*pin, &request.op, &peripherals.gpio),
        BusRef::I2c { bus, addr } => handle_i2c(*bus, *addr, &request.op, &peripherals.handles.i2c),
        BusRef::Spi { bus, chip_select } => {
            handle_spi(*bus, *chip_select, &request.op, &peripherals.handles.spi)
        }
        BusRef::Uart { port, baud_rate } => {
            handle_uart(*port, *baud_rate, &request.op, &peripherals.handles.uart)
        }
    }
}

/// セッションが実際に保持しているバスだけを解放する。`LockTable`が所有者を
/// 再確認した状態でキャッシュをdropし、その後にロックを削除するため、非所有者の
/// Releaseや次クライアントとの競合で使用中FDを閉じることはない。
fn release_owned_bus<I, S, U>(
    locks: &LockTable,
    handles: &PeripheralHandles<I, S, U>,
    client_id: &ClientId,
    held_buses: &mut HashSet<BusId>,
    bus: BusId,
) -> bool {
    if !held_buses.remove(&bus) {
        return false;
    }
    locks.release_with(bus, client_id, || {
        handles.close(bus);
    })
}

fn pull_mode_from_wire(pull: PullWire) -> PullMode {
    match pull {
        PullWire::None => PullMode::None,
        PullWire::Up => PullMode::Up,
        PullWire::Down => PullMode::Down,
    }
}

fn handle_gpio(pin: u32, op: &Operation, gpio: &Mutex<GpioChip>) -> Response {
    match op {
        Operation::Read { .. } | Operation::Write { .. } => {
            let mut chip = gpio.lock().expect("gpio mutex poisoned");
            let result = match op {
                Operation::Read { pull } => chip
                    .claim_input(pin, pull_mode_from_wire(*pull))
                    .and_then(|()| chip.read(pin))
                    .map(|level| level == Level::High),
                Operation::Write { value } => {
                    let level = if *value { Level::High } else { Level::Low };
                    chip.claim_output(pin)
                        .and_then(|()| chip.write(pin, level))
                        .map(|()| *value)
                }
                _ => unreachable!(),
            };
            match result {
                Ok(value) => Response::value(value),
                Err(err) => Response::hw_error(&err.to_string()),
            }
        }
        Operation::WatchEdges {
            pre_pulse_low_ms,
            max_events,
            timeout_ms,
            pull,
        } => handle_watch_edges(
            pin,
            *pre_pulse_low_ms,
            *max_events,
            *timeout_ms,
            *pull,
            gpio,
        ),
        Operation::WatchEdgesPolled {
            pre_pulse_low_ms,
            budget_ms,
            idle_timeout_us,
            glitch_filter_us,
            pull,
        } => handle_watch_edges_polled(
            pin,
            *pre_pulse_low_ms,
            *budget_ms,
            *idle_timeout_us,
            *glitch_filter_us,
            *pull,
            gpio,
        ),
        Operation::ReadBytes { .. }
        | Operation::WriteBytes { .. }
        | Operation::WriteReadBytes { .. }
        | Operation::Transfer { .. } => Response::malformed("gpioバスにはバイト列操作は使えません"),
        Operation::Release => unreachable!("Releaseはdispatchの時点で処理済み"),
    }
}

/// スタート信号（任意）を送ってからエッジを記録する（Tier 2、DHT22向け）。
///
/// `pre_pulse_low_ms`が指定されていれば、`/dev/gpiomem`経由（Tier 1、
/// `gpio.rs`）でピンをLOW出力にしてから待ち、その後`/dev/gpiochip0`経由
/// （Tier 2、`gpio_watch.rs`）に切り替えてエッジ監視を開始する。両者は
/// カーネルのピン使用状況把握という点で別経路のため、この切り替え自体は
/// カーネル側の衝突検知（EBUSY）の対象にならない（`gpio_watch.rs`のモジュール
/// docを参照）。このピンの`LockTable`ロックは呼び出し元がスタート信号から
/// 監視終了まで保持しているため、他クライアントの割り込みは防がれている。
fn handle_watch_edges(
    pin: u32,
    pre_pulse_low_ms: Option<u64>,
    max_events: usize,
    timeout_ms: u64,
    pull: PullWire,
    gpio: &Mutex<GpioChip>,
) -> Response {
    if let Some(ms) = pre_pulse_low_ms {
        let mut chip = gpio.lock().expect("gpio mutex poisoned");
        let result = chip
            .claim_output(pin)
            .and_then(|()| chip.write(pin, Level::Low));
        drop(chip); // sleep中は他バスのGPIO操作をブロックしない。
        if let Err(err) = result {
            return Response::hw_error(&err.to_string());
        }
        std::thread::sleep(Duration::from_millis(ms));
    }

    let watcher = EdgeWatcher::open(GPIOCHIP_PATH, pin, pull_mode_from_wire(pull));
    match watcher.and_then(|mut w| w.wait_events(Duration::from_millis(timeout_ms), max_events)) {
        Ok(events) => Response::edges(
            events
                .into_iter()
                .map(|e| EdgeEventWire {
                    timestamp_ns: e.timestamp_ns,
                    rising: e.rising,
                })
                .collect(),
        ),
        Err(err) => Response::hw_error(&err.to_string()),
    }
}

/// `WatchEdges`（カーネルGPIO v2エッジ割り込み）の代替。割り込みに
/// 頼らず、`/dev/gpiomem`の生レベルを高速busy-loopで連続サンプリングし、
/// レベルが変化した瞬間をエッジとして記録する。終了判定は反復回数ではなく
/// 呼び出し側が指定した実時間を使うため、信号プロトコルやCPU速度へ固定されない。
/// 戻り値の形式（`edges`）は`WatchEdges`と同一。
fn handle_watch_edges_polled(
    pin: u32,
    pre_pulse_low_ms: Option<u64>,
    budget_ms: u64,
    idle_timeout_us: u64,
    glitch_filter_us: u64,
    pull: PullWire,
    gpio: &Mutex<GpioChip>,
) -> Response {
    let mut chip = gpio.lock().expect("gpio mutex poisoned");

    if let Some(ms) = pre_pulse_low_ms {
        let result = chip
            .claim_output(pin)
            .and_then(|()| chip.write(pin, Level::Low));
        if let Err(err) = result {
            return Response::hw_error(&err.to_string());
        }
        drop(chip);
        std::thread::sleep(Duration::from_millis(ms));
        chip = gpio.lock().expect("gpio mutex poisoned");
    }

    if let Err(err) = chip.claim_input(pin, pull_mode_from_wire(pull)) {
        return Response::hw_error(&err.to_string());
    }

    let deadline = Instant::now() + Duration::from_millis(budget_ms);
    let idle_timeout = Duration::from_micros(idle_timeout_us);
    let glitch_filter = Duration::from_micros(glitch_filter_us);
    let mut idle_deadline = Instant::now() + idle_timeout;
    let mut events: Vec<EdgeEventWire> = Vec::new();
    let mut edge_filter = PolledEdgeFilter::new(glitch_filter);

    loop {
        let now = Instant::now();
        if now >= deadline || now >= idle_deadline {
            break;
        }
        let level = match chip.read(pin) {
            Ok(level) => level,
            Err(err) => return Response::hw_error(&err.to_string()),
        };
        if let Some(event) = edge_filter.observe(level, now, monotonic_now_ns()) {
            events.push(event);
            idle_deadline = Instant::now() + idle_timeout;
        }
    }

    Response::edges(events)
}

struct PolledEdgeFilter {
    minimum_stable: Duration,
    accepted: Option<Level>,
    candidate: Option<(Level, Instant, u64)>,
}

impl PolledEdgeFilter {
    fn new(minimum_stable: Duration) -> Self {
        Self {
            minimum_stable,
            accepted: None,
            candidate: None,
        }
    }

    fn observe(&mut self, level: Level, now: Instant, timestamp_ns: u64) -> Option<EdgeEventWire> {
        let Some(accepted) = self.accepted else {
            self.accepted = Some(level);
            return None;
        };
        if level == accepted {
            self.candidate = None;
            return None;
        }
        if self.minimum_stable.is_zero() {
            self.accepted = Some(level);
            return Some(EdgeEventWire {
                timestamp_ns,
                rising: level == Level::High,
            });
        }

        match self.candidate {
            Some((candidate, since, edge_timestamp_ns)) if candidate == level => {
                if now.duration_since(since) >= self.minimum_stable {
                    self.accepted = Some(level);
                    self.candidate = None;
                    Some(EdgeEventWire {
                        timestamp_ns: edge_timestamp_ns,
                        rising: level == Level::High,
                    })
                } else {
                    None
                }
            }
            _ => {
                self.candidate = Some((level, now, timestamp_ns));
                None
            }
        }
    }
}

fn handle_i2c(bus_num: u8, addr: u8, op: &Operation, i2c: &Mutex<I2cBuses>) -> Response {
    let mut buses = i2c.lock().expect("i2c mutex poisoned");
    let bus = match buses.entry(bus_num) {
        Entry::Occupied(entry) => entry.into_mut(),
        Entry::Vacant(entry) => match I2cBus::open(bus_num) {
            Ok(opened) => entry.insert(opened),
            Err(err) => return Response::hw_error(&err.to_string()),
        },
    };

    match op {
        Operation::ReadBytes { length } => {
            let mut buf = vec![0u8; *length];
            match bus.read(addr, &mut buf) {
                Ok(()) => Response::bytes(buf),
                Err(err) => Response::hw_error(&err.to_string()),
            }
        }
        Operation::WriteBytes { data } => match bus.write(addr, data) {
            Ok(()) => Response::ok(),
            Err(err) => Response::hw_error(&err.to_string()),
        },
        Operation::WriteReadBytes { data, length } => {
            let mut buf = vec![0u8; *length];
            match bus.write_read(addr, data, &mut buf) {
                Ok(()) => Response::bytes(buf),
                Err(err) => Response::hw_error(&err.to_string()),
            }
        }
        Operation::Read { .. }
        | Operation::Write { .. }
        | Operation::Transfer { .. }
        | Operation::WatchEdges { .. }
        | Operation::WatchEdgesPolled { .. } => {
            Response::malformed("i2cバスにはこの操作は使えません")
        }
        Operation::Release => unreachable!("Releaseはdispatchの時点で処理済み"),
    }
}

fn handle_spi(bus_num: u8, chip_select: u8, op: &Operation, spi: &Mutex<SpiDevices>) -> Response {
    let mut devices = spi.lock().expect("spi mutex poisoned");
    let device = match devices.entry((bus_num, chip_select)) {
        Entry::Occupied(entry) => entry.into_mut(),
        Entry::Vacant(entry) => match SpiDevice::open(bus_num, chip_select) {
            Ok(opened) => entry.insert(opened),
            Err(err) => return Response::hw_error(&err.to_string()),
        },
    };

    match op {
        Operation::Transfer { data } => {
            let mut rx = vec![0u8; data.len()];
            match device.transfer(data, &mut rx) {
                Ok(()) => Response::bytes(rx),
                Err(err) => Response::hw_error(&err.to_string()),
            }
        }
        Operation::Read { .. }
        | Operation::Write { .. }
        | Operation::ReadBytes { .. }
        | Operation::WriteBytes { .. }
        | Operation::WriteReadBytes { .. }
        | Operation::WatchEdges { .. }
        | Operation::WatchEdgesPolled { .. } => {
            Response::malformed("spiバスにはこの操作は使えません")
        }
        Operation::Release => unreachable!("Releaseはdispatchの時点で処理済み"),
    }
}

fn handle_uart(port: u8, baud_rate: u32, op: &Operation, uart: &Mutex<UartPorts>) -> Response {
    let mut ports = uart.lock().expect("uart mutex poisoned");
    let device_path = format!("/dev/ttyS{port}");
    let opened = match ports.entry(port) {
        Entry::Occupied(entry) => entry.into_mut(),
        Entry::Vacant(entry) => match UartPort::open(&device_path, baud_rate) {
            Ok(opened) => entry.insert(opened),
            Err(err) => return Response::hw_error(&err.to_string()),
        },
    };

    match op {
        Operation::ReadBytes { length } => {
            let mut buf = vec![0u8; *length];
            match opened.read(&mut buf) {
                Ok(n) => Response::bytes(buf[..n].to_vec()),
                Err(err) => Response::hw_error(&err.to_string()),
            }
        }
        Operation::WriteBytes { data } => match opened.write(data) {
            Ok(_) => Response::ok(),
            Err(err) => Response::hw_error(&err.to_string()),
        },
        Operation::Read { .. }
        | Operation::Write { .. }
        | Operation::WriteReadBytes { .. }
        | Operation::Transfer { .. }
        | Operation::WatchEdges { .. }
        | Operation::WatchEdgesPolled { .. } => {
            Response::malformed("uartバスにはこの操作は使えません")
        }
        Operation::Release => unreachable!("Releaseはdispatchの時点で処理済み"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener as StdUnixListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct DropProbe(Arc<AtomicUsize>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn client(pid: u32, session_id: u64) -> ClientId {
        ClientId::Local {
            uid: 1000,
            pid,
            session_id,
        }
    }

    fn temporary_socket_path(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "pi4gpio-{label}-{}-{unique}.sock",
            std::process::id()
        ))
    }

    #[test]
    fn second_daemon_cannot_unlink_active_socket() {
        let path = temporary_socket_path("active");
        let listener = StdUnixListener::bind(&path).expect("bind active socket");

        let err = remove_stale_socket(&path).expect_err("active socket must be preserved");
        assert_eq!(err.kind(), io::ErrorKind::AddrInUse);
        assert!(path.exists());

        drop(listener);
        std::fs::remove_file(path).expect("remove test socket");
    }

    #[test]
    fn stale_socket_is_removed_before_bind() {
        let path = temporary_socket_path("stale");
        let listener = StdUnixListener::bind(&path).expect("bind stale socket");
        drop(listener);

        remove_stale_socket(&path).expect("remove stale socket");
        assert!(!path.exists());
    }

    #[test]
    fn non_socket_path_is_never_replaced() {
        let path = temporary_socket_path("regular");
        std::fs::write(&path, b"keep").expect("write regular file");

        let err = remove_stale_socket(&path).expect_err("regular path must be preserved");
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&path).expect("read regular file"), b"keep");

        std::fs::remove_file(path).expect("remove regular file");
    }

    #[test]
    fn explicit_release_drops_handle_before_next_owner_acquires() {
        let locks = LockTable::new();
        let handles: PeripheralHandles<DropProbe, DropProbe, DropProbe> =
            PeripheralHandles::default();
        let drops = Arc::new(AtomicUsize::new(0));
        let owner = client(10, 1);
        let next = client(20, 2);
        let bus = BusId::I2c(1);
        let mut held = HashSet::from([bus]);

        handles
            .i2c
            .lock()
            .unwrap()
            .insert(1, DropProbe(Arc::clone(&drops)));
        assert_eq!(locks.try_acquire(bus, owner.clone()), Ok(()));
        assert!(release_owned_bus(&locks, &handles, &owner, &mut held, bus));

        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(held.is_empty());
        assert!(handles.i2c.lock().unwrap().is_empty());
        assert_eq!(locks.try_acquire(bus, next), Ok(()));
    }

    #[test]
    fn disconnect_cleanup_drops_all_cached_bus_handles() {
        let locks = LockTable::new();
        let handles: PeripheralHandles<DropProbe, DropProbe, DropProbe> =
            PeripheralHandles::default();
        let drops = Arc::new(AtomicUsize::new(0));
        let owner = client(10, 1);
        let buses = [BusId::I2c(1), BusId::Spi(0, 0), BusId::Uart(0)];
        let mut held = HashSet::from(buses);

        handles
            .i2c
            .lock()
            .unwrap()
            .insert(1, DropProbe(Arc::clone(&drops)));
        handles
            .spi
            .lock()
            .unwrap()
            .insert((0, 0), DropProbe(Arc::clone(&drops)));
        handles
            .uart
            .lock()
            .unwrap()
            .insert(0, DropProbe(Arc::clone(&drops)));
        for bus in buses {
            assert_eq!(locks.try_acquire(bus, owner.clone()), Ok(()));
        }

        for bus in buses {
            assert!(release_owned_bus(&locks, &handles, &owner, &mut held, bus));
        }

        assert_eq!(drops.load(Ordering::SeqCst), 3);
        assert!(held.is_empty());
        assert!(handles.i2c.lock().unwrap().is_empty());
        assert!(handles.spi.lock().unwrap().is_empty());
        assert!(handles.uart.lock().unwrap().is_empty());
    }

    #[test]
    fn non_owner_release_cannot_drop_cached_handle() {
        let locks = LockTable::new();
        let handles: PeripheralHandles<DropProbe, DropProbe, DropProbe> =
            PeripheralHandles::default();
        let drops = Arc::new(AtomicUsize::new(0));
        let owner = client(10, 1);
        let contender = client(20, 2);
        let bus = BusId::Uart(0);
        // 不整合なheld集合まで想定し、LockTable側の所有者確認も検証する。
        let mut contender_held = HashSet::from([bus]);

        handles
            .uart
            .lock()
            .unwrap()
            .insert(0, DropProbe(Arc::clone(&drops)));
        assert_eq!(locks.try_acquire(bus, owner.clone()), Ok(()));
        assert!(!release_owned_bus(
            &locks,
            &handles,
            &contender,
            &mut contender_held,
            bus,
        ));

        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert!(handles.uart.lock().unwrap().contains_key(&0));
        assert_eq!(locks.try_acquire(bus, contender), Err(owner));
    }

    #[test]
    fn polled_edge_filter_rejects_short_glitch_and_preserves_edge_timestamp() {
        let start = Instant::now();
        let mut filter = PolledEdgeFilter::new(Duration::from_micros(10));

        assert!(filter.observe(Level::Low, start, 100).is_none());
        assert!(filter
            .observe(Level::High, start + Duration::from_micros(1), 200)
            .is_none());
        assert!(filter
            .observe(Level::Low, start + Duration::from_micros(5), 300)
            .is_none());

        assert!(filter
            .observe(Level::High, start + Duration::from_micros(20), 400)
            .is_none());
        let edge = filter
            .observe(Level::High, start + Duration::from_micros(31), 500)
            .expect("stable edge");
        assert_eq!(edge.timestamp_ns, 400);
        assert!(edge.rising);
    }
}
