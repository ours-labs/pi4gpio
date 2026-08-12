//! Full-duplex SPI transfers through the Linux `spidev` interface.

use crate::error::HwError;
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;

const DEFAULT_SPEED_HZ: u32 = 1_000_000;
const DEFAULT_BITS_PER_WORD: u8 = 8;

#[repr(C)]
struct SpiIocTransfer {
    tx_buf: u64,
    rx_buf: u64,
    len: u32,
    speed_hz: u32,
    delay_usecs: u16,
    bits_per_word: u8,
    cs_change: u8,
    tx_nbits: u8,
    rx_nbits: u8,
    pad: u16,
}

const IOC_NRBITS: u32 = 8;
const IOC_TYPEBITS: u32 = 8;
const IOC_SIZEBITS: u32 = 14;
const IOC_NRSHIFT: u32 = 0;
const IOC_TYPESHIFT: u32 = IOC_NRSHIFT + IOC_NRBITS;
const IOC_SIZESHIFT: u32 = IOC_TYPESHIFT + IOC_TYPEBITS;
const IOC_DIRSHIFT: u32 = IOC_SIZESHIFT + IOC_SIZEBITS;
const IOC_WRITE: u32 = 1;

const fn ioc(dir: u32, ty: u32, nr: u32, size: u32) -> libc::c_ulong {
    ((dir << IOC_DIRSHIFT) | (ty << IOC_TYPESHIFT) | (nr << IOC_NRSHIFT) | (size << IOC_SIZESHIFT))
        as libc::c_ulong
}

const SPI_IOC_MAGIC: u32 = b'k' as u32;
const SPI_IOC_TRANSFER_SIZE: usize = std::mem::size_of::<SpiIocTransfer>();
const _: () = assert!(SPI_IOC_TRANSFER_SIZE == 32);
const SPI_IOC_MESSAGE_1: libc::c_ulong =
    ioc(IOC_WRITE, SPI_IOC_MAGIC, 0, SPI_IOC_TRANSFER_SIZE as u32);
/// `SPI_IOC_WR_MODE` (`__u8`).
const SPI_IOC_WR_MODE: libc::c_ulong = ioc(IOC_WRITE, SPI_IOC_MAGIC, 1, 1);

pub struct SpiDevice {
    file: File,
}

impl SpiDevice {
    pub fn open(bus: u8, chip_select: u8) -> Result<Self, HwError> {
        let path = format!("/dev/spidev{bus}.{chip_select}");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| HwError::OpenFailed(format!("{path}: {e}")))?;

        let mode: u8 = 0;
        let result = unsafe {
            libc::ioctl(
                file.as_raw_fd(),
                SPI_IOC_WR_MODE,
                &mode as *const u8 as *mut u8,
            )
        };
        if result < 0 {
            return Err(HwError::OpenFailed(format!(
                "SPI_IOC_WR_MODE ioctl: {}",
                std::io::Error::last_os_error()
            )));
        }

        Ok(Self { file })
    }

    pub fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<(), HwError> {
        if tx.len() != rx.len() {
            return Err(HwError::TransferFailed(format!(
                "tx/rx length mismatch: tx={}, rx={}",
                tx.len(),
                rx.len()
            )));
        }

        let xfer = SpiIocTransfer {
            tx_buf: tx.as_ptr() as u64,
            rx_buf: rx.as_mut_ptr() as u64,
            len: tx.len() as u32,
            speed_hz: DEFAULT_SPEED_HZ,
            delay_usecs: 0,
            bits_per_word: DEFAULT_BITS_PER_WORD,
            cs_change: 0,
            tx_nbits: 0,
            rx_nbits: 0,
            pad: 0,
        };

        let result = unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                SPI_IOC_MESSAGE_1,
                &xfer as *const SpiIocTransfer,
            )
        };

        if result < 0 {
            return Err(HwError::TransferFailed(format!(
                "SPI_IOC_MESSAGE ioctl: {}",
                std::io::Error::last_os_error()
            )));
        }
        Ok(())
    }
}
