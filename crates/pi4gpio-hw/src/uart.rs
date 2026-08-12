//! Raw 8N1 UART access through the Linux termios interface.

use crate::error::HwError;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;

pub struct UartPort {
    file: File,
}

impl UartPort {
    pub fn open(device: &str, baud_rate: u32) -> Result<Self, HwError> {
        let speed = Self::baud_to_speed(baud_rate)?;

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY)
            .open(device)
            .map_err(|e| HwError::OpenFailed(format!("{device}: {e}")))?;

        let mut termios: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(file.as_raw_fd(), &mut termios) } < 0 {
            return Err(HwError::OpenFailed(format!(
                "tcgetattr {device}: {}",
                std::io::Error::last_os_error()
            )));
        }

        unsafe {
            libc::cfsetispeed(&mut termios, speed);
            libc::cfsetospeed(&mut termios, speed);
        }

        termios.c_cflag &= !(libc::PARENB | libc::CSTOPB | libc::CSIZE);
        termios.c_cflag |= libc::CS8 | libc::CLOCAL | libc::CREAD;
        termios.c_iflag &=
            !(libc::IXON | libc::IXOFF | libc::IXANY | libc::ICRNL | libc::INLCR | libc::IGNBRK);
        termios.c_oflag &= !libc::OPOST;
        termios.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ECHOE | libc::ISIG | libc::IEXTEN);
        termios.c_cc[libc::VMIN] = 0;
        termios.c_cc[libc::VTIME] = 10;

        if unsafe { libc::tcsetattr(file.as_raw_fd(), libc::TCSANOW, &termios) } < 0 {
            return Err(HwError::OpenFailed(format!(
                "tcsetattr {device}: {}",
                std::io::Error::last_os_error()
            )));
        }

        Ok(Self { file })
    }

    fn baud_to_speed(baud: u32) -> Result<libc::speed_t, HwError> {
        Ok(match baud {
            1_200 => libc::B1200,
            2_400 => libc::B2400,
            4_800 => libc::B4800,
            9_600 => libc::B9600,
            19_200 => libc::B19200,
            38_400 => libc::B38400,
            57_600 => libc::B57600,
            115_200 => libc::B115200,
            _ => return Err(HwError::InvalidChannel(baud)),
        })
    }

    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, HwError> {
        self.file
            .read(buf)
            .map_err(|e| HwError::TransferFailed(format!("uart read: {e}")))
    }

    pub fn write(&mut self, data: &[u8]) -> Result<usize, HwError> {
        self.file
            .write(data)
            .map_err(|e| HwError::TransferFailed(format!("uart write: {e}")))
    }
}
