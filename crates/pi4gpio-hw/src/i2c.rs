//! I2C access through the Linux `i2c-dev` combined-transaction interface.

use crate::error::HwError;
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;

const I2C_RDWR: libc::c_ulong = 0x0707;
const I2C_M_RD: u16 = 0x0001;

const MAX_ADDR: u8 = 0x7f;

#[repr(C)]
struct I2cMsg {
    addr: u16,
    flags: u16,
    len: u16,
    buf: *mut u8,
}

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
