//! BCM2711 GPIO access through the restricted `/dev/gpiomem` mapping.

use crate::error::HwError;
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::ptr;

const GPIO_MEM_PATH: &str = "/dev/gpiomem";
const GPIO_BLOCK_SIZE: usize = 4096;

const GPFSEL0: usize = 0; // 0x00
const GPSET0: usize = 0x1c / 4;
const GPCLR0: usize = 0x28 / 4;
const GPLEV0: usize = 0x34 / 4;
const GPPUPPDN0: usize = 0xe4 / 4;

const MAX_PIN: u32 = 57;

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

unsafe impl Send for GpioChip {}

impl GpioChip {
    pub fn open() -> Result<Self, HwError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(GPIO_MEM_PATH)
            .map_err(|e| HwError::OpenFailed(format!("{GPIO_MEM_PATH}: {e}")))?;

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
    unsafe fn read_reg(&self, word_offset: usize) -> u32 {
        unsafe { ptr::read_volatile(self.mem.add(word_offset)) }
    }

    /// # Safety
    unsafe fn write_reg(&self, word_offset: usize, value: u32) {
        unsafe { ptr::write_volatile(self.mem.add(word_offset), value) }
    }

    fn set_function(&mut self, pin: u32, func: Function) {
        let reg = GPFSEL0 + (pin as usize / 10);
        let shift = (pin % 10) * 3;
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
        unsafe {
            self.write_reg(reg, 1 << bit);
        }
        Ok(())
    }

    pub fn read(&self, pin: u32) -> Result<Level, HwError> {
        Self::check_pin(pin)?;
        let reg = GPLEV0 + (pin as usize / 32);
        let bit = pin % 32;
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
        unsafe {
            libc::munmap(self.mem as *mut libc::c_void, GPIO_BLOCK_SIZE);
        }
    }
}
