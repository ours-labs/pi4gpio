//! SPIハードウェアアクセス（FEATURE_PRIORITY.md Tier 1）。
//!
//! I2Cと同じ理由で、カーネルの`spidev`（`/dev/spidevB.D`へのioctl）経由で
//! 実装する。全二重の`SPI_IOC_MESSAGE`ioctlで1回のCSアサーション内での
//! 送受信を行う——Pythonの`spidev.SpiDev.xfer2()`と同じ挙動。
//!
//! 現在はmode 0と固定default speedを提供する。request単位のmode・speed設定は
//! protocolとownership contractを定義してから追加する。

use crate::error::HwError;
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;

/// Conservative default suitable for common low-speed ADC transactions.
const DEFAULT_SPEED_HZ: u32 = 1_000_000;
const DEFAULT_BITS_PER_WORD: u8 = 8;

/// `linux/spi/spidev.h`の`struct spi_ioc_transfer`と同じレイアウト（32バイト）。
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

// Linuxのioctl番号エンコーディング（asm-generic/ioctl.h）に沿って算出する。
// マジックナンバーを直書きせず導出することで、datasheet/kernel headerと
// 突き合わせて検算できるようにしている。
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
// linux/spi/spidev.hのstruct spi_ioc_transferは32バイト固定。ここが
// ずれるとioctlの`size`フィールドが誤り、カーネル側で弾かれるか
// 未定義動作になるため、レイアウト変更を静かに見逃さないよう検証する。
const _: () = assert!(SPI_IOC_TRANSFER_SIZE == 32);
/// `SPI_IOC_MESSAGE(1)`。この実装は常に1メッセージ（1回のCSアサーション）
/// のみを送るため、可変長ではなく固定値として持つ。
const SPI_IOC_MESSAGE_1: libc::c_ulong =
    ioc(IOC_WRITE, SPI_IOC_MAGIC, 0, SPI_IOC_TRANSFER_SIZE as u32);
/// `SPI_IOC_WR_MODE`（`__u8`）。
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

        // モード0固定。デバイスの以前の利用者が別モードへ変更している
        // 可能性があるため、毎回明示的に設定する。
        let mode: u8 = 0;
        // SAFETY: `mode`はこのブロックの間有効な`u8`へのポインタ。
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

    /// `tx`と同じ長さだけ送受信する全二重転送（`spidev.SpiDev.xfer2()`相当）。
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

        // SAFETY: `xfer.tx_buf`/`rx_buf`は呼び出し元の`tx`/`rx`スライスを指す
        // 有効なポインタで、このioctl呼び出しが完了するまで生存している。
        // `rx`は排他的に借用されているため他から同時に変更されない。
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
