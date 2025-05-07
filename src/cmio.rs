use std::os::unix::io::RawFd;
use std::ptr;
use libc::{self, c_void, ioctl, mmap, munmap, MAP_FAILED, MAP_SHARED, PROT_READ, PROT_WRITE};
use thiserror::Error;

const CMIO_DEVICE: &str = "/dev/cmio";
const IOCTL_CMIO_SETUP: libc::c_ulong = 0xd3 << 16 | 0;
const IOCTL_CMIO_YIELD: libc::c_ulong = 0xd3 << 16 | 1;

#[repr(C)]
pub struct CmioBuffer {
    pub data: u64,
    pub length: u64,
}

#[repr(C)]
pub struct CmioSetup {
    pub tx: CmioBuffer,
    pub rx: CmioBuffer,
}

#[repr(C)]
pub struct CmioYield {
    pub dev: u8,
    pub cmd: u8,
    pub reason: u16,
    pub data: u32,
}

#[derive(Error, Debug)]
pub enum CmioError {
    #[error("Failed to open CMIO device: {0}")]
    OpenError(#[from] std::io::Error),
    #[error("Failed to setup CMIO: {0}")]
    SetupError(i32),
    #[error("Failed to map memory: {0}")]
    MapError(i32),
    #[error("Buffer too large: {0} bytes (max: {1})")]
    BufferTooLarge(usize, usize),
}

pub struct Cmio {
    fd: RawFd,
    tx_buffer: *mut c_void,
    rx_buffer: *mut c_void,
    tx_length: usize,
    rx_length: usize,
}

impl Cmio {
    pub fn new() -> Result<Self, CmioError> {
        let fd = unsafe {
            libc::open(
                CMIO_DEVICE.as_ptr() as *const libc::c_char,
                libc::O_RDWR,
                0,
            )
        };

        if fd < 0 {
            return Err(CmioError::OpenError(std::io::Error::last_os_error()));
        }

        let mut setup = CmioSetup {
            tx: CmioBuffer { data: 0, length: 0 },
            rx: CmioBuffer { data: 0, length: 0 },
        };

        if unsafe { ioctl(fd, IOCTL_CMIO_SETUP, &mut setup) } < 0 {
            return Err(CmioError::SetupError(std::io::Error::last_os_error().raw_os_error().unwrap()));
        }

        let tx_buffer = unsafe {
            mmap(
                setup.tx.data as *mut c_void,
                setup.tx.length as usize,
                PROT_READ | PROT_WRITE,
                MAP_SHARED,
                fd,
                0,
            )
        };

        if tx_buffer == MAP_FAILED {
            return Err(CmioError::MapError(std::io::Error::last_os_error().raw_os_error().unwrap()));
        }

        let rx_buffer = unsafe {
            mmap(
                setup.rx.data as *mut c_void,
                setup.rx.length as usize,
                PROT_READ,
                MAP_SHARED,
                fd,
                0,
            )
        };

        if rx_buffer == MAP_FAILED {
            unsafe { munmap(tx_buffer, setup.tx.length as usize) };
            return Err(CmioError::MapError(std::io::Error::last_os_error().raw_os_error().unwrap()));
        }

        Ok(Self {
            fd,
            tx_buffer,
            rx_buffer,
            tx_length: setup.tx.length as usize,
            rx_length: setup.rx.length as usize,
        })
    }

    pub fn yield_(&mut self, yield_data: &mut CmioYield) -> Result<(), CmioError> {
        let packed = ((yield_data.dev as u64) << 56)
            | ((yield_data.cmd as u64) << 48)
            | ((yield_data.reason as u64) << 32)
            | (yield_data.data as u64);

        let mut req = packed;
        if unsafe { ioctl(self.fd, IOCTL_CMIO_YIELD, &mut req) } < 0 {
            return Err(CmioError::SetupError(std::io::Error::last_os_error().raw_os_error().unwrap()));
        }

        yield_data.dev = (req >> 56) as u8;
        yield_data.cmd = (req >> 48) as u8;
        yield_data.reason = (req >> 32) as u16;
        yield_data.data = req as u32;

        Ok(())
    }

    /// Convenience function to yield with a buffer and get the response
    /// 
    /// # Arguments
    /// 
    /// * `dev` - Device ID
    /// * `cmd` - Command ID
    /// * `reason` - Reason code
    /// * `tx_data` - Data to send in the TX buffer
    /// 
    /// # Returns
    /// 
    /// * `Ok((Vec<u8>, u16))` - A tuple containing the data received in the RX buffer and the reason code
    /// * `Err(CmioError)` - If an error occurs
    pub fn yield_with_buffer(
        &mut self,
        dev: u8,
        cmd: u8,
        reason: u16,
        tx_data: &[u8],
    ) -> Result<(Vec<u8>, u16), CmioError> {
        // Check if the buffer is too large
        if tx_data.len() > self.tx_length {
            return Err(CmioError::BufferTooLarge(tx_data.len(), self.tx_length));
        }

        // Copy data to TX buffer
        unsafe {
            ptr::copy_nonoverlapping(
                tx_data.as_ptr(),
                self.tx_buffer as *mut u8,
                tx_data.len(),
            );
        }

        // Create yield data with the length of the data
        let mut yield_data = CmioYield {
            dev,
            cmd,
            reason,
            data: tx_data.len() as u32,
        };

        // Perform the yield
        self.yield_(&mut yield_data)?;

        // Get the length of the response data
        let rx_length = yield_data.data as usize;
        
        // Check if the response is too large
        if rx_length > self.rx_length {
            return Err(CmioError::BufferTooLarge(rx_length, self.rx_length));
        }

        // Copy data from RX buffer
        let mut rx_data = vec![0u8; rx_length];
        unsafe {
            ptr::copy_nonoverlapping(
                self.rx_buffer as *const u8,
                rx_data.as_mut_ptr(),
                rx_length,
            );
        }

        Ok((rx_data, yield_data.reason))
    }

    /// Get the maximum size of the TX buffer
    pub fn get_tx_length(&self) -> usize {
        self.tx_length
    }
}

impl Drop for Cmio {
    fn drop(&mut self) {
        unsafe {
            munmap(self.tx_buffer, self.tx_length);
            munmap(self.rx_buffer, self.rx_length);
            libc::close(self.fd);
        }
    }
} 