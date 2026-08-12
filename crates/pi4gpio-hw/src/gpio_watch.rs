//! GPIOエッジ検出/通知（FEATURE_PRIORITY.md Tier 2）。
//!
//! Tier 1のGPIO（`gpio.rs`、`/dev/gpiomem`直叩き）とは別経路で、カーネルの
//! `gpiochip`キャラクタデバイス（GPIO v2 uAPI、`linux/gpio.h`）を使う。
//! ポーリングではなく本物の割り込み駆動で、カーネルが記録したタイムスタンプ
//! （`CLOCK_MONOTONIC`）付きのエッジイベントを受け取れるため、DHT22のような
//! マイクロ秒単位のパルス幅判定にポーリングループより向いている。
//! I2C/SPI/UARTで「カーネルドライバに委ねる」と判断したのと同じ理由で、
//! userspace DMA schedulingの複雑さをedge notification pathへ持ち込まずに済む。
//!
//! `/dev/gpiomem`（`gpio.rs`）と`/dev/gpiochipN`（このモジュール）は、
//! カーネルのピン使用状況の把握という点で別経路である点に注意。`gpiomem`
//! への書き込みはカーネルの`gpiochip`管理からは見えないため、同じピンを
//! Tier 1の書き込みとTier 2の監視で使い分けても、`gpiochip`側がEBUSYで
//! 衝突検知することはない。複数クライアント間の排他はpi4gpio-daemon側の
//! `LockTable`が担う。
//!
//! 構造体レイアウトはLinux GPIO v2 uAPI headerに従い、`const assert`で
//! sizeのずれを検知する。

use crate::error::HwError;
use crate::gpio::PullMode;
use std::fs::{File, OpenOptions};
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::time::{Duration, Instant};

const GPIO_MAX_NAME_SIZE: usize = 32;
const GPIO_V2_LINES_MAX: usize = 64;
const GPIO_V2_LINE_NUM_ATTRS_MAX: usize = 10;

const GPIO_V2_LINE_FLAG_INPUT: u64 = 1 << 2;
const GPIO_V2_LINE_FLAG_EDGE_RISING: u64 = 1 << 4;
const GPIO_V2_LINE_FLAG_EDGE_FALLING: u64 = 1 << 5;
const GPIO_V2_LINE_FLAG_BIAS_PULL_UP: u64 = 1 << 8;
const GPIO_V2_LINE_FLAG_BIAS_PULL_DOWN: u64 = 1 << 9;
const GPIO_V2_LINE_FLAG_BIAS_DISABLED: u64 = 1 << 10;

/// BCM2711はGPIO0〜57の58本（gpio.rsのMAX_PINと同じ値）。
const MAX_PIN: u32 = 57;

/// `union { flags; values; debounce_period_us }`を、実際に使うのが`flags`
/// (u64)のみなのでu64一枚として表現している（サイズは実物のunionと一致）。
#[repr(C)]
struct GpioV2LineAttribute {
    id: u32,
    padding: u32,
    value: u64,
}

#[repr(C)]
struct GpioV2LineConfigAttribute {
    attr: GpioV2LineAttribute,
    mask: u64,
}

#[repr(C)]
struct GpioV2LineConfig {
    flags: u64,
    num_attrs: u32,
    padding: [u32; 5],
    attrs: [GpioV2LineConfigAttribute; GPIO_V2_LINE_NUM_ATTRS_MAX],
}

#[repr(C)]
struct GpioV2LineRequest {
    offsets: [u32; GPIO_V2_LINES_MAX],
    consumer: [u8; GPIO_MAX_NAME_SIZE],
    config: GpioV2LineConfig,
    num_lines: u32,
    event_buffer_size: u32,
    padding: [u32; 5],
    fd: i32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct GpioV2LineEvent {
    timestamp_ns: u64,
    id: u32,
    offset: u32,
    seqno: u32,
    line_seqno: u32,
    padding: [u32; 6],
}

// /usr/include/linux/gpio.h (このPiで実測済み) のバイト数と一致することを検証。
const _: () = assert!(std::mem::size_of::<GpioV2LineRequest>() == 592);
const _: () = assert!(std::mem::size_of::<GpioV2LineEvent>() == 48);

// asm-generic/ioctl.hのビットパッキング。spi.rsと同じ導出方法。
const IOC_NRBITS: u32 = 8;
const IOC_TYPEBITS: u32 = 8;
const IOC_SIZEBITS: u32 = 14;
const IOC_NRSHIFT: u32 = 0;
const IOC_TYPESHIFT: u32 = IOC_NRSHIFT + IOC_NRBITS;
const IOC_SIZESHIFT: u32 = IOC_TYPESHIFT + IOC_TYPEBITS;
const IOC_DIRSHIFT: u32 = IOC_SIZESHIFT + IOC_SIZEBITS;
const IOC_READ: u32 = 2;
const IOC_WRITE: u32 = 1;

const fn ioc(dir: u32, ty: u32, nr: u32, size: u32) -> libc::c_ulong {
    ((dir << IOC_DIRSHIFT) | (ty << IOC_TYPESHIFT) | (nr << IOC_NRSHIFT) | (size << IOC_SIZESHIFT))
        as libc::c_ulong
}

const GPIO_IOC_MAGIC: u32 = 0xb4;
/// `GPIO_V2_GET_LINE_IOCTL`（`_IOWR(0xB4, 0x07, struct gpio_v2_line_request)`）。
const GPIO_V2_GET_LINE_IOCTL: libc::c_ulong = ioc(
    IOC_READ | IOC_WRITE,
    GPIO_IOC_MAGIC,
    0x07,
    std::mem::size_of::<GpioV2LineRequest>() as u32,
);

pub struct EdgeEvent {
    pub timestamp_ns: u64,
    pub rising: bool,
}

pub struct EdgeWatcher {
    line: File,
}

impl EdgeWatcher {
    /// `pull`: DHT22のような、センサー側がオープンドレインでバスを操作する
    /// プロトコルでは、モジュールに外部プルアップが無い場合SoC側のプルアップ
    /// が必要になる（`PullMode::Up`を指定する）。gpio.rsのTier 1 Readが
    /// pullを選べるようにしたのと同じ理由。
    pub fn open(chip_path: &str, pin: u32, pull: PullMode) -> Result<Self, HwError> {
        if pin > MAX_PIN {
            return Err(HwError::InvalidChannel(pin));
        }

        let chip = OpenOptions::new()
            .read(true)
            .open(chip_path)
            .map_err(|e| HwError::OpenFailed(format!("{chip_path}: {e}")))?;

        let bias_flag = match pull {
            PullMode::None => GPIO_V2_LINE_FLAG_BIAS_DISABLED,
            PullMode::Up => GPIO_V2_LINE_FLAG_BIAS_PULL_UP,
            PullMode::Down => GPIO_V2_LINE_FLAG_BIAS_PULL_DOWN,
        };

        // SAFETY: ゼロ初期化はこの構造体の全フィールド（配列含む）にとって
        // 有効なビットパターン（0）を作る。
        let mut request: GpioV2LineRequest = unsafe { std::mem::zeroed() };
        request.offsets[0] = pin;
        request.num_lines = 1;
        let consumer = b"pi4gpio\0";
        request.consumer[..consumer.len()].copy_from_slice(consumer);
        request.config.flags = GPIO_V2_LINE_FLAG_INPUT
            | GPIO_V2_LINE_FLAG_EDGE_RISING
            | GPIO_V2_LINE_FLAG_EDGE_FALLING
            | bias_flag;

        // SAFETY: `request`は上で有効な値に初期化済み。`chip`のfdはこの
        // ブロックの間有効。成功時、カーネルが`request.fd`に新しい
        // 行リクエスト用のfd（このプロセスが所有する）を書き込む。
        let result = unsafe {
            libc::ioctl(
                chip.as_raw_fd(),
                GPIO_V2_GET_LINE_IOCTL,
                &mut request as *mut GpioV2LineRequest,
            )
        };
        if result < 0 {
            return Err(HwError::OpenFailed(format!(
                "GPIO_V2_GET_LINE_IOCTL {chip_path} pin={pin}: {}",
                std::io::Error::last_os_error()
            )));
        }

        // SAFETY: `request.fd`はカーネルがGPIO_V2_GET_LINE_IOCTL成功時に
        // 発行した、このプロセスが排他的に所有する有効なfd。`chip`自体は
        // この後dropしてよい（行リクエストのfdは独立して有効）。
        let line = unsafe { File::from_raw_fd(request.fd as RawFd) };
        Ok(Self { line })
    }

    /// `timeout`まで待ち、発生したエッジイベントを最大`max_events`件返す。
    /// タイムアウトした場合はその時点までに集まったイベント（空の場合あり）を返す。
    pub fn wait_events(
        &mut self,
        timeout: Duration,
        max_events: usize,
    ) -> Result<Vec<EdgeEvent>, HwError> {
        let deadline = Instant::now() + timeout;
        let mut events = Vec::new();

        while events.len() < max_events {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }

            let mut pfd = libc::pollfd {
                fd: self.line.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: `pfd`は1要素のスタック上の値で、この呼び出しの間有効。
            let poll_result =
                unsafe { libc::poll(&mut pfd, 1, remaining.as_millis() as libc::c_int) };
            if poll_result < 0 {
                return Err(HwError::TransferFailed(format!(
                    "poll: {}",
                    std::io::Error::last_os_error()
                )));
            }
            if poll_result == 0 {
                break; // タイムアウト
            }

            let mut raw = GpioV2LineEvent::default();
            let event_size = std::mem::size_of::<GpioV2LineEvent>();
            // SAFETY: `raw`はevent_sizeバイトの書き込み先として妥当な、
            // 排他的に借用されたスタック上の値。
            let n = unsafe {
                libc::read(
                    self.line.as_raw_fd(),
                    &mut raw as *mut GpioV2LineEvent as *mut libc::c_void,
                    event_size,
                )
            };
            if n < 0 {
                return Err(HwError::TransferFailed(format!(
                    "gpio line event read: {}",
                    std::io::Error::last_os_error()
                )));
            }
            if n as usize != event_size {
                return Err(HwError::TransferFailed(format!(
                    "gpio line event read: short read ({n} bytes, expected {event_size})"
                )));
            }

            events.push(EdgeEvent {
                timestamp_ns: raw.timestamp_ns,
                rising: raw.id == 1, // GPIO_V2_LINE_EVENT_RISING_EDGE
            });
        }

        Ok(events)
    }
}

/// `CLOCK_MONOTONIC`基準のナノ秒タイムスタンプ。GPIO v2イベントの
/// `timestamp_ns`（`GPIO_V2_LINE_FLAG_EVENT_CLOCK_REALTIME`を指定しない限り
/// カーネルは`CLOCK_MONOTONIC`基準で打刻する）と同じ時刻源になるため、
/// Tier 1の高速ポーリング（`pi4gpio-daemon`の`handle_watch_edges_polled`）
/// で合成するタイムスタンプも、Tier 2のカーネルタイムスタンプと同じ土俵で
/// 扱える。デコード側（`_decode_dht22_edges`等）はどちらの経路で得た
/// エッジ列かを区別する必要が無い。
pub fn monotonic_now_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts`はこの呼び出しの間有効なスタック上の値。`CLOCK_MONOTONIC`
    // はLinuxで常にサポートされる。
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
    }
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}
