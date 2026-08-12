//! I2Cハードウェアアクセス（FEATURE_PRIORITY.md Tier 1）。
//!
//! カーネルのi2c-dev（`/dev/i2c-N`へのioctl）経由で実装する。GPIOと違い
//! BSC(Broadcom Serial Controller)のレジスタを直接叩かない設計とした——
//! I2Cはクロックストレッチング・マルチマスター調停・NAK処理などプロトコル
//! 自体が複雑で、これを自前で再実装するリスクは、既に実績あるカーネル
//! ドライバに委ねるメリットに見合わない。デーモンが複数クライアントの
//! 排他制御を担うという価値提供（`LockTable`）はこの方式でも変わらず
//! 成立する。pigpio本家もハードウェアI2Cでは同じくi2c-dev ioctlを使う。
//!
//! Register-oriented devices often require a combined transaction that writes
//! a register pointer and reads without an intervening STOP, so `write_read` is
//! provided separately from simple `read` and `write` operations.

use crate::error::HwError;
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;

/// `linux/i2c-dev.h`のI2C_RDWR ioctl番号。
const I2C_RDWR: libc::c_ulong = 0x0707;
/// `linux/i2c.h`のI2C_M_RDフラグ（読み取り方向のメッセージであることを示す）。
const I2C_M_RD: u16 = 0x0001;

/// 標準的な7bit I2Cアドレスの上限（0x00〜0x07・0x78〜0x7Fは予約領域）。
const MAX_ADDR: u8 = 0x7f;

/// `linux/i2c.h`の`struct i2c_msg`と同じレイアウト。
#[repr(C)]
struct I2cMsg {
    addr: u16,
    flags: u16,
    len: u16,
    buf: *mut u8,
}

/// `linux/i2c-dev.h`の`struct i2c_rdwr_ioctl_data`と同じレイアウト。
#[repr(C)]
struct I2cRdwrIoctlData {
    msgs: *mut I2cMsg,
    nmsgs: u32,
}

pub struct I2cBus {
    file: File,
}

impl I2cBus {
    pub fn open(bus: u8) -> Result<Self, HwError> {
        let path = format!("/dev/i2c-{bus}");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| HwError::OpenFailed(format!("{path}: {e}")))?;
        Ok(Self { file })
    }

    fn check_addr(addr: u8) -> Result<(), HwError> {
        if addr > MAX_ADDR {
            return Err(HwError::InvalidChannel(addr as u32));
        }
        Ok(())
    }

    pub fn read(&mut self, addr: u8, buf: &mut [u8]) -> Result<(), HwError> {
        Self::check_addr(addr)?;
        let mut msg = I2cMsg {
            addr: addr as u16,
            flags: I2C_M_RD,
            len: buf.len() as u16,
            buf: buf.as_mut_ptr(),
        };
        self.transfer(std::slice::from_mut(&mut msg))
    }

    pub fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), HwError> {
        Self::check_addr(addr)?;
        let mut msg = I2cMsg {
            addr: addr as u16,
            flags: 0,
            len: data.len() as u16,
            buf: data.as_ptr().cast_mut(),
        };
        self.transfer(std::slice::from_mut(&mut msg))
    }

    /// `data`を書き込んだ直後、STOPを挟まずリピートスタートで`buf`分を読み取る。
    /// BME280のようなレジスタポインタ方式のセンサーで必須のパターン。
    pub fn write_read(&mut self, addr: u8, data: &[u8], buf: &mut [u8]) -> Result<(), HwError> {
        Self::check_addr(addr)?;
        let mut msgs = [
            I2cMsg {
                addr: addr as u16,
                flags: 0,
                len: data.len() as u16,
                buf: data.as_ptr().cast_mut(),
            },
            I2cMsg {
                addr: addr as u16,
                flags: I2C_M_RD,
                len: buf.len() as u16,
                buf: buf.as_mut_ptr(),
            },
        ];
        self.transfer(&mut msgs)
    }

    fn transfer(&mut self, msgs: &mut [I2cMsg]) -> Result<(), HwError> {
        let mut rdwr = I2cRdwrIoctlData {
            msgs: msgs.as_mut_ptr(),
            nmsgs: msgs.len() as u32,
        };

        // SAFETY: `rdwr`は`msgs`を指し、各`I2cMsg.buf`は呼び出し元が渡した
        // スライスを指す有効なポインタ。このioctl呼び出しが完了するまで
        // `msgs`および参照先バッファは生存しており、書き込み先バッファは
        // 排他的に借用されているため他から同時に変更されない。
        let result = unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                I2C_RDWR,
                &mut rdwr as *mut I2cRdwrIoctlData,
            )
        };

        if result < 0 {
            return Err(HwError::TransferFailed(format!(
                "I2C_RDWR ioctl: {}",
                std::io::Error::last_os_error()
            )));
        }
        Ok(())
    }
}
