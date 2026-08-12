//! Timestamped GPIO edge capture through the Linux GPIO v2 character API.
//!
//! Event timestamps and polled timestamps use `CLOCK_MONOTONIC`.

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

const MAX_PIN: u32 = 57;

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

const _: () = assert!(std::mem::size_of::<GpioV2LineRequest>() == 592);
const _: () = assert!(std::mem::size_of::<GpioV2LineEvent>() == 48);

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
/// `GPIO_V2_GET_LINE_IOCTL` (`_IOWR(0xB4, 0x07, struct gpio_v2_line_request)`).
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

        let mut request: GpioV2LineRequest = unsafe { std::mem::zeroed() };
        request.offsets[0] = pin;
        request.num_lines = 1;
        let consumer = b"pi4gpio\0";
        request.consumer[..consumer.len()].copy_from_slice(consumer);
        request.config.flags = GPIO_V2_LINE_FLAG_INPUT
            | GPIO_V2_LINE_FLAG_EDGE_RISING
            | GPIO_V2_LINE_FLAG_EDGE_FALLING
            | bias_flag;

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

        let line = unsafe { File::from_raw_fd(request.fd as RawFd) };
        Ok(Self { line })
    }

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
            let poll_result =
                unsafe { libc::poll(&mut pfd, 1, remaining.as_millis() as libc::c_int) };
            if poll_result < 0 {
                return Err(HwError::TransferFailed(format!(
                    "poll: {}",
                    std::io::Error::last_os_error()
                )));
            }
            if poll_result == 0 {
                break;
            }

            let mut raw = GpioV2LineEvent::default();
            let event_size = std::mem::size_of::<GpioV2LineEvent>();
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

pub fn monotonic_now_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
    }
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}
