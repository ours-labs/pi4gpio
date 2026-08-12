//! GPIO基本読み書き（FEATURE_PRIORITY.md Tier 1）。
//!
//! `/dev/gpiomem`（GPIOレジスタのページのみを公開する、Raspberry Pi専用の
//! キャラクタデバイス）をmmapし、BCM2711のGPIOレジスタに直接読み書きする。
//! `/dev/mem`と違い露出範囲がGPIOレジスタに限定されるため、root不要
//! （`gpio`グループのメンバーであればよい）かつ他の物理アドレス空間への
//! 誤アクセスのリスクが無い。DMA制御ブロック等、GPIO以外のペリフェラルに
//! 触れる必要が出た時点（Tier 3以降）で`/dev/mem`への切り替えを検討する。
//!
//! レジスタオフセットはBCM2711 ARM Peripherals datasheetに準拠
//! （BCM2835系から変更されていないGPFSEL/GPSET/GPCLR/GPLEVと、
//! BCM2711で追加されたGPIO_PUP_PDN_CNTRL_REGn）。

use crate::error::HwError;
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::ptr;

const GPIO_MEM_PATH: &str = "/dev/gpiomem";
/// mmapするサイズ。使用する全レジスタ（最大オフセット0xF0）はこの範囲に収まる。
const GPIO_BLOCK_SIZE: usize = 4096;

// ワードオフセット（4バイト単位）。バイトオフセットはdatasheet記載値。
const GPFSEL0: usize = 0; // 0x00
const GPSET0: usize = 0x1c / 4;
const GPCLR0: usize = 0x28 / 4;
const GPLEV0: usize = 0x34 / 4;
const GPPUPPDN0: usize = 0xe4 / 4;

/// BCM2711はGPIO0〜57の58本。
const MAX_PIN: u32 = 57;

// 実機検証で判明: GPIO_PUP_PDN_CNTRL_REGnのビット値は
// 0b01=プルダウン・0b10=プルアップ（BCM2835旧世代のGPPUDとは逆順）。
// GPIO17で claim_input(Up)->Low / claim_input(Down)->High
// （きれいに入れ替わった結果）が観測され、この対応が確定した。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PullMode {
    None = 0b00,
    Down = 0b01,
    Up = 0b10,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Low,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
enum Function {
    Input = 0b000,
    Output = 0b001,
}

pub struct GpioChip {
    mem: *mut u32,
    _file: File,
}

// SAFETY: `mem`はこのプロセスが排他的にmmapした領域であり、`GpioChip`の
// メソッド経由でのみアクセスされる。ポインタそのものをスレッド間で
// 共有しても、指す先のメモリ操作自体はvolatile読み書きでアトミック性を
// 必要としないため（レジスタ単位でRead-Modify-Writeが競合しない設計は
// 呼び出し側の責務）、`Send`は安全。
unsafe impl Send for GpioChip {}

impl GpioChip {
    pub fn open() -> Result<Self, HwError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(GPIO_MEM_PATH)
            .map_err(|e| HwError::OpenFailed(format!("{GPIO_MEM_PATH}: {e}")))?;

        // SAFETY: `/dev/gpiomem`はRaspberry Pi専用のキャラクタデバイスで、
        // GPIOレジスタのページのみを公開する。fdはこのブロックの間有効。
        let addr = unsafe {
            libc::mmap(
                ptr::null_mut(),
                GPIO_BLOCK_SIZE,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };

        if addr == libc::MAP_FAILED {
            return Err(HwError::OpenFailed(format!(
                "mmap {GPIO_MEM_PATH}: {}",
                std::io::Error::last_os_error()
            )));
        }

        Ok(Self {
            mem: addr as *mut u32,
            _file: file,
        })
    }

    fn check_pin(pin: u32) -> Result<(), HwError> {
        if pin > MAX_PIN {
            return Err(HwError::InvalidChannel(pin));
        }
        Ok(())
    }

    /// # Safety
    /// `word_offset`はこのモジュール内の定数から導出され、常に
    /// `GPIO_BLOCK_SIZE`（1ページ）内に収まっていなければならない。
    unsafe fn read_reg(&self, word_offset: usize) -> u32 {
        unsafe { ptr::read_volatile(self.mem.add(word_offset)) }
    }

    /// # Safety
    /// `read_reg`と同様、`word_offset`の範囲に注意すること。
    unsafe fn write_reg(&self, word_offset: usize, value: u32) {
        unsafe { ptr::write_volatile(self.mem.add(word_offset), value) }
    }

    fn set_function(&mut self, pin: u32, func: Function) {
        let reg = GPFSEL0 + (pin as usize / 10);
        let shift = (pin % 10) * 3;
        // SAFETY: pinはcheck_pin済み（0..=57）。GPFSEL0..5の6ワード
        // （reg = GPFSEL0 + 0..=5）はGPIO_BLOCK_SIZE内に収まる。
        unsafe {
            let mut value = self.read_reg(reg);
            value &= !(0b111 << shift);
            value |= (func as u32) << shift;
            self.write_reg(reg, value);
        }
    }

    fn set_pull(&mut self, pin: u32, pull: PullMode) {
        let reg = GPPUPPDN0 + (pin as usize / 16);
        let shift = (pin % 16) * 2;
        // SAFETY: pinはcheck_pin済み。GPPUPPDN0..3の4ワードは
        // GPIO_BLOCK_SIZE内に収まる。
        unsafe {
            let mut value = self.read_reg(reg);
            value &= !(0b11 << shift);
            value |= (pull as u32) << shift;
            self.write_reg(reg, value);
        }
    }

    pub fn claim_output(&mut self, pin: u32) -> Result<(), HwError> {
        Self::check_pin(pin)?;
        self.set_function(pin, Function::Output);
        Ok(())
    }

    pub fn claim_input(&mut self, pin: u32, pull: PullMode) -> Result<(), HwError> {
        Self::check_pin(pin)?;
        self.set_function(pin, Function::Input);
        self.set_pull(pin, pull);
        Ok(())
    }

    pub fn write(&mut self, pin: u32, level: Level) -> Result<(), HwError> {
        Self::check_pin(pin)?;
        let reg_base = if level == Level::High { GPSET0 } else { GPCLR0 };
        let reg = reg_base + (pin as usize / 32);
        let bit = pin % 32;
        // SAFETY: pinはcheck_pin済み。GPSET0/1・GPCLR0/1はGPIO_BLOCK_SIZE内。
        unsafe {
            self.write_reg(reg, 1 << bit);
        }
        Ok(())
    }

    pub fn read(&self, pin: u32) -> Result<Level, HwError> {
        Self::check_pin(pin)?;
        let reg = GPLEV0 + (pin as usize / 32);
        let bit = pin % 32;
        // SAFETY: pinはcheck_pin済み。GPLEV0/1はGPIO_BLOCK_SIZE内。
        let value = unsafe { self.read_reg(reg) };
        Ok(if value & (1 << bit) != 0 {
            Level::High
        } else {
            Level::Low
        })
    }
}

impl Drop for GpioChip {
    fn drop(&mut self) {
        // SAFETY: `mem`と`GPIO_BLOCK_SIZE`は`open`でmmapした領域そのもの。
        unsafe {
            libc::munmap(self.mem as *mut libc::c_void, GPIO_BLOCK_SIZE);
        }
    }
}
