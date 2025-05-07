use std::io;
use libc;
use tun_tap::{Iface, Mode};
use crate::cmio::{Cmio, CmioError};

// HTIF yield constants
const HTIF_DEVICE_YIELD: u8 = 0x02;
const HTIF_YIELD_CMD_MANUAL: u8 = 0x01;
const TAP_RXTX_CMD: u16 = 0x42;

// Buffer sizes
const MAX_PACKET_SIZE: usize = 1500; // Standard MTU size

pub struct NetworkInterface {
    cmio: Cmio,
    iface: Iface,
    read_buffer: Vec<u8>,
    cmio_max_buffer_size: usize,
}

impl NetworkInterface {
    pub fn new() -> Result<Self, CmioError> {
        // Initialize CMIO
        let cmio = Cmio::new()?;
        
        // Get the CMIO max buffer size from the CMIO instance
        let cmio_max_buffer_size = cmio.get_tx_length();
        
        // Create a TAP interface
        let iface = Iface::new("tapcmio0", Mode::Tap)
            .map_err(|e| CmioError::SetupError(e.raw_os_error().unwrap_or(-1)))?;
        
        // Set up buffer for reading
        let read_buffer = vec![0u8; MAX_PACKET_SIZE];
        
        Ok(Self {
            cmio,
            iface,
            read_buffer,
            cmio_max_buffer_size,
        })
    }
    
    /// Run the network interface loop
    /// 
    /// This function implements the main loop for the network interface:
    /// 1. Read as many frames as possible from the TAP interface
    /// 2. Batch them into CMIO transmissions with length prefixes
    /// 3. Process received data by injecting frames one at a time into the TAP interface
    /// 4. Try to read more frames from CMIO until we get a zero-length response
    /// 5. Yield to the scheduler when there's no more data to process
    pub fn run_loop(&mut self) -> Result<(), CmioError> {
        loop {
            // Step 1: Read as many frames as possible from the TAP interface
            let packets = self.get_packets_to_transmit()?;
            
            if !packets.is_empty() {
                // Step 2: Batch packets into CMIO-sized chunks and send them
                
                // Create batches of packets that fit within CMIO buffer size
                let mut current_batch = Vec::new();
                let mut current_batch_size = 0;
                
                for packet in packets {
                    // Calculate the size of this packet with its length prefix
                    let packet_size = packet.len() + 2; // 2 bytes for length prefix
                    
                    // Check if adding this packet would exceed the CMIO buffer size
                    if current_batch_size + packet_size > self.cmio_max_buffer_size && !current_batch.is_empty() {
                        // Send the current batch
                        self.send_batch(&current_batch)?;
                        
                        // Start a new batch
                        current_batch = Vec::new();
                        current_batch_size = 0;
                    }
                    
                    // Add the packet to the current batch
                    current_batch.push(packet);
                    current_batch_size += packet_size;
                }
                
                // Send any remaining packets in the last batch
                if !current_batch.is_empty() {
                    self.send_batch(&current_batch)?;
                }
                
                // Step 4: Try to read more frames from CMIO until we get a zero-length response
                loop {
                    let (rx_data, _reason) = self.cmio.yield_with_buffer(
                        HTIF_DEVICE_YIELD,
                        HTIF_YIELD_CMD_MANUAL,
                        TAP_RXTX_CMD,
                        &[],
                    )?;
                    
                    if rx_data.is_empty() {
                        // No more data to receive, break the inner loop
                        break;
                    }
                    
                    self.process_received_data(&rx_data)?;
                }
            } else {
                // No data to transmit, check for incoming data
                let (rx_data, _reason) = self.cmio.yield_with_buffer(
                    HTIF_DEVICE_YIELD,
                    HTIF_YIELD_CMD_MANUAL,
                    TAP_RXTX_CMD,
                    &[],
                )?;
                
                // Process received data if any
                if !rx_data.is_empty() {
                    self.process_received_data(&rx_data)?;
                    
                    // Try to read more frames from CMIO until we get a zero-length response
                    loop {
                        let (rx_data, _reason) = self.cmio.yield_with_buffer(
                            HTIF_DEVICE_YIELD,
                            HTIF_YIELD_CMD_MANUAL,
                            TAP_RXTX_CMD,
                            &[],
                        )?;
                        
                        if rx_data.is_empty() {
                            // No more data to receive, break the inner loop
                            break;
                        }
                        
                        self.process_received_data(&rx_data)?;
                    }
                } else {
                    // Step 5: No data to transmit or receive, yield to the scheduler
                    // Use HTIF yield device with manual yield command and TAP_RXTX_CMD reason
                    self.cmio.yield_with_buffer(
                        HTIF_DEVICE_YIELD,
                        HTIF_YIELD_CMD_MANUAL,
                        TAP_RXTX_CMD,
                        &[],
                    )?;
                }
            }
        }
    }
    
    /// Get packets to transmit from the network interface
    /// 
    /// This function reads multiple packets from the TAP interface and returns them
    /// as a vector of individual packets.
    fn get_packets_to_transmit(&mut self) -> Result<Vec<Vec<u8>>, CmioError> {
        let mut packets = Vec::new();
        
        loop {
            // Try to read a packet using recv
            match self.iface.recv(&mut self.read_buffer) {
                Ok(n) => {
                    if n > 0 {
                        // We have data to transmit
                        // Create a new packet buffer and copy the data
                        let mut packet = vec![0u8; n];
                        packet.copy_from_slice(&self.read_buffer[..n]);
                        packets.push(packet);
                    } else {
                        // No more data available
                        break;
                    }
                },
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        // No more data available, non-blocking read
                        break;
                    } else {
                        // Some other error occurred
                        return Err(CmioError::SetupError(e.raw_os_error().unwrap_or(-1)));
                    }
                }
            }
        }
        
        // Return the packets
        Ok(packets)
    }
    
    /// Send a batch of packets via CMIO
    /// 
    /// This function takes a vector of packets, adds length prefixes to each,
    /// and sends them as a single batch via CMIO.
    fn send_batch(&mut self, packets: &[Vec<u8>]) -> Result<(), CmioError> {
        // Create a buffer for the batched data
        let mut batch_buffer = Vec::new();
        
        // Add each packet with its length prefix
        for packet in packets {
            // Add u16 length prefix (network byte order)
            let length_bytes = (packet.len() as u16).to_be_bytes();
            batch_buffer.extend_from_slice(&length_bytes);
            
            // Add the packet data
            batch_buffer.extend_from_slice(packet);
        }
        
        // Send the batched data via CMIO
        let (rx_data, _reason) = self.cmio.yield_with_buffer(
            HTIF_DEVICE_YIELD,
            HTIF_YIELD_CMD_MANUAL,
            TAP_RXTX_CMD,
            &batch_buffer,
        )?;
        
        // Process received data if any
        if !rx_data.is_empty() {
            self.process_received_data(&rx_data)?;
        }
        
        Ok(())
    }
    
    /// Process received data and write it to the network interface
    /// 
    /// This function processes received data that may contain multiple packets,
    /// each prefixed with a u16 length, and writes them to the TAP interface.
    fn process_received_data(&mut self, data: &[u8]) -> Result<(), CmioError> {
        let mut offset = 0;
        
        // Process each packet in the batch
        while offset < data.len() {
            // Check if we have enough data for the length prefix
            if offset + 2 > data.len() {
                break;
            }
            
            // Read the length prefix (network byte order)
            let length_bytes = [data[offset], data[offset + 1]];
            let packet_length = u16::from_be_bytes(length_bytes) as usize;
            offset += 2;
            
            // Check if we have enough data for the packet
            if offset + packet_length > data.len() {
                break;
            }
            
            // Extract the packet data
            let packet_data = &data[offset..offset + packet_length];
            
            // Write the packet to the TAP interface using send
            self.iface.send(packet_data)
                .map_err(|e| CmioError::SetupError(e.raw_os_error().unwrap_or(-1)))?;
            
            // Move to the next packet
            offset += packet_length;
        }
        
        Ok(())
    }
}

// No need for a custom Drop implementation as Iface implements Drop 