use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::net::TcpStream;
use crate::cmio::{Cmio, CmioError};

// HTIF yield constants
const HTIF_DEVICE_YIELD: u8 = 0x02;
const HTIF_YIELD_CMD_MANUAL: u8 = 0x01;
const UNIX_SOCKET_CMD: u16 = 0x43;

// Message types
const MSG_TYPE_UNIX_CONNECT: u8 = 0x01;
const MSG_TYPE_UNIX_SEND: u8 = 0x02;
const MSG_TYPE_UNIX_RECEIVE: u8 = 0x03;
const MSG_TYPE_UNIX_CLOSE: u8 = 0x04;
const MSG_TYPE_TCP_CONNECT: u8 = 0x05;
const MSG_TYPE_TCP_SEND: u8 = 0x06;
const MSG_TYPE_TCP_RECEIVE: u8 = 0x07;
const MSG_TYPE_TCP_CLOSE: u8 = 0x08;

// Maximum path length for Unix domain socket
const MAX_PATH_LENGTH: usize = 108;

// Structure for socket messages
#[derive(Debug, Clone)]
struct SocketMessage {
    msg_type: u8,
    socket_id: u32,
    path: String,
    ip_addr: [u8; 4],
    port: u16,
    data: Vec<u8>,
}

impl SocketMessage {
    fn new(msg_type: u8, socket_id: u32, path: String, ip_addr: [u8; 4], port: u16, data: Vec<u8>) -> Self {
        Self {
            msg_type,
            socket_id,
            path,
            ip_addr,
            port,
            data,
        }
    }

    fn serialize(&self) -> Vec<u8> {
        let mut buffer = Vec::new();
        
        // Add message type
        buffer.push(self.msg_type);
        
        // Add socket ID (4 bytes, network byte order)
        buffer.extend_from_slice(&self.socket_id.to_be_bytes());
        
        // Only include connection info for connect messages, and only the relevant info
        match self.msg_type {
            MSG_TYPE_UNIX_CONNECT => {
                // Add path length (as u8)
                buffer.push(self.path.len() as u8);
                // Add path
                buffer.extend_from_slice(self.path.as_bytes());
            },
            MSG_TYPE_TCP_CONNECT => {
                // Add IP address (4 bytes)
                buffer.extend_from_slice(&self.ip_addr);
                // Add port (2 bytes, network byte order)
                buffer.extend_from_slice(&self.port.to_be_bytes());
            },
            _ => {
                // For non-connect messages, add data length and data
                let data_len = self.data.len() as u32;
                buffer.extend_from_slice(&data_len.to_be_bytes());
                buffer.extend_from_slice(&self.data);
            }
        }
        
        buffer
    }

    fn deserialize(data: &[u8]) -> Result<Self, CmioError> {
        if data.len() < 5 { // 1 (type) + 4 (socket_id)
            return Err(CmioError::SetupError(-1)); // Invalid message format
        }
        
        let msg_type = data[0];
        
        // Read socket ID (4 bytes, network byte order)
        let socket_id_bytes = [data[1], data[2], data[3], data[4]];
        let socket_id = u32::from_be_bytes(socket_id_bytes);
        
        let mut offset = 5;
        let mut path = String::new();
        let mut ip_addr = [0u8; 4];
        let mut port = 0u16;
        let mut message_data = Vec::new();
        
        // Only read connection info for connect messages, and only the relevant info
        match msg_type {
            MSG_TYPE_UNIX_CONNECT => {
                if data.len() < offset + 1 {
                    return Err(CmioError::SetupError(-1)); // Invalid message format
                }
                
                let path_len = data[offset] as usize;
                offset += 1;
                
                if data.len() < offset + path_len {
                    return Err(CmioError::SetupError(-1)); // Invalid message format
                }
                
                let path_bytes = &data[offset..offset + path_len];
                path = String::from_utf8(path_bytes.to_vec())
                    .map_err(|_| CmioError::SetupError(-1))?;
                offset += path_len;
            },
            MSG_TYPE_TCP_CONNECT => {
                if data.len() < offset + 6 { // 4 (ip) + 2 (port)
                    return Err(CmioError::SetupError(-1)); // Invalid message format
                }
                
                // Read IP address (4 bytes)
                ip_addr = [data[offset], data[offset + 1], data[offset + 2], data[offset + 3]];
                offset += 4;
                
                // Read port (2 bytes, network byte order)
                let port_bytes = [data[offset], data[offset + 1]];
                port = u16::from_be_bytes(port_bytes);
                offset += 2;
            },
            _ => {
                if data.len() < offset + 4 {
                    return Err(CmioError::SetupError(-1)); // Invalid message format
                }
                
                // Read data length (u32, network byte order)
                let data_len_bytes = [data[offset], data[offset + 1], data[offset + 2], data[offset + 3]];
                let data_len = u32::from_be_bytes(data_len_bytes) as usize;
                offset += 4;
                
                if data.len() < offset + data_len {
                    return Err(CmioError::SetupError(-1)); // Invalid message format
                }
                
                message_data = data[offset..offset + data_len].to_vec();
            }
        }
        
        Ok(Self {
            msg_type,
            socket_id,
            path,
            ip_addr,
            port,
            data: message_data,
        })
    }
}

// Structure to manage socket connections
pub struct SocketManager {
    cmio: Arc<Mutex<Cmio>>,
    unix_connections: Arc<Mutex<HashMap<u32, (String, UnixStream)>>>,
    tcp_connections: Arc<Mutex<HashMap<u32, (String, TcpStream)>>>,
    cmio_max_buffer_size: usize,
}

impl SocketManager {
    pub fn new(cmio: Cmio, cmio_max_buffer_size: usize) -> Self {
        Self {
            cmio: Arc::new(Mutex::new(cmio)),
            unix_connections: Arc::new(Mutex::new(HashMap::new())),
            tcp_connections: Arc::new(Mutex::new(HashMap::new())),
            cmio_max_buffer_size,
        }
    }
    
    pub fn run_loop(&self) -> Result<(), CmioError> {
        loop {
            // Check for incoming messages
            let (rx_data, _reason) = {
                let mut cmio = self.cmio.lock().unwrap();
                cmio.yield_with_buffer(
                    HTIF_DEVICE_YIELD,
                    HTIF_YIELD_CMD_MANUAL,
                    UNIX_SOCKET_CMD,
                    &[],
                )?
            };
            
            if !rx_data.is_empty() {
                // Process the received data
                self.process_received_data(&rx_data)?;
            } else {
                // No data to receive, yield to the scheduler
                let mut cmio = self.cmio.lock().unwrap();
                cmio.yield_with_buffer(
                    HTIF_DEVICE_YIELD,
                    HTIF_YIELD_CMD_MANUAL,
                    UNIX_SOCKET_CMD,
                    &[],
                )?;
            }
        }
    }
    
    fn process_received_data(&self, data: &[u8]) -> Result<(), CmioError> {
        let mut offset = 0;
        let mut responses = Vec::new();
        
        // Process each message in the batch
        while offset < data.len() {
            // Try to deserialize a message
            match SocketMessage::deserialize(&data[offset..]) {
                Ok(message) => {
                    // Process the message based on its type
                    let response = match message.msg_type {
                        MSG_TYPE_UNIX_CONNECT => self.handle_unix_connect(message.clone()),
                        MSG_TYPE_UNIX_SEND => self.handle_unix_send(message.clone()),
                        MSG_TYPE_UNIX_RECEIVE => self.handle_unix_receive(message.clone()),
                        MSG_TYPE_UNIX_CLOSE => self.handle_unix_close(message.clone()),
                        MSG_TYPE_TCP_CONNECT => self.handle_tcp_connect(message.clone()),
                        MSG_TYPE_TCP_SEND => self.handle_tcp_send(message.clone()),
                        MSG_TYPE_TCP_RECEIVE => self.handle_tcp_receive(message.clone()),
                        MSG_TYPE_TCP_CLOSE => self.handle_tcp_close(message.clone()),
                        _ => Err(CmioError::SetupError(-1)), // Unknown message type
                    }?;
                    
                    // Add the response to our batch
                    responses.extend_from_slice(&response);
                    
                    // Calculate the size of the processed message
                    let msg_size = 1 + 4 + 4 + message.data.len();
                    offset += msg_size;
                },
                Err(e) => {
                    // Error deserializing message, stop processing
                    return Err(e);
                }
            }
        }
        
        // Send all responses in a single CMIO transmission
        if !responses.is_empty() {
            let mut cmio = self.cmio.lock().unwrap();
            cmio.yield_with_buffer(
                HTIF_DEVICE_YIELD,
                HTIF_YIELD_CMD_MANUAL,
                UNIX_SOCKET_CMD,
                &responses,
            )?;
        }
        
        Ok(())
    }
    
    fn handle_unix_connect(&self, message: SocketMessage) -> Result<Vec<u8>, CmioError> {
        // Connect to the Unix domain socket
        let stream = UnixStream::connect(Path::new(&message.path))
            .map_err(|e| CmioError::SetupError(e.raw_os_error().unwrap_or(-1)))?;
        
        // Add the connection to our map
        {
            let mut connections = self.unix_connections.lock().unwrap();
            connections.insert(message.socket_id, (message.path.clone(), stream));
        }
        
        // Return success response
        Ok(SocketMessage::new(
            MSG_TYPE_UNIX_CONNECT,
            message.socket_id,
            message.path,
            message.ip_addr,
            message.port,
            vec![0], // Success
        ).serialize())
    }
    
    fn handle_unix_send(&self, message: SocketMessage) -> Result<Vec<u8>, CmioError> {
        // Find the connection
        let mut connections = self.unix_connections.lock().unwrap();
        let connection = connections.get_mut(&message.socket_id);
        
        match connection {
            Some((_, stream)) => {
                // Write data to the socket
                stream.write_all(&message.data)
                    .map_err(|e| CmioError::SetupError(e.raw_os_error().unwrap_or(-1)))?;
                
                // Return success response
                Ok(SocketMessage::new(
                    MSG_TYPE_UNIX_SEND,
                    message.socket_id,
                    message.path,
                    message.ip_addr,
                    message.port,
                    vec![0], // Success
                ).serialize())
            },
            None => {
                // Connection not found
                Ok(SocketMessage::new(
                    MSG_TYPE_UNIX_SEND,
                    message.socket_id,
                    message.path,
                    message.ip_addr,
                    message.port,
                    vec![1], // Error: Connection not found
                ).serialize())
            }
        }
    }
    
    fn handle_unix_receive(&self, message: SocketMessage) -> Result<Vec<u8>, CmioError> {
        // Find the connection
        let mut connections = self.unix_connections.lock().unwrap();
        let connection = connections.get_mut(&message.socket_id);
        
        match connection {
            Some((_, stream)) => {
                // Read data from the socket
                let mut buffer = vec![0u8; 4096]; // Read up to 4KB
                match stream.read(&mut buffer) {
                    Ok(n) => {
                        // Return the received data
                        Ok(SocketMessage::new(
                            MSG_TYPE_UNIX_RECEIVE,
                            message.socket_id,
                            message.path,
                            message.ip_addr,
                            message.port,
                            buffer[..n].to_vec(),
                        ).serialize())
                    },
                    Err(e) => {
                        if e.kind() == io::ErrorKind::WouldBlock {
                            // No data available
                            Ok(SocketMessage::new(
                                MSG_TYPE_UNIX_RECEIVE,
                                message.socket_id,
                                message.path,
                                message.ip_addr,
                                message.port,
                                vec![], // Empty data
                            ).serialize())
                        } else {
                            // Error reading from socket
                            Err(CmioError::SetupError(e.raw_os_error().unwrap_or(-1)))
                        }
                    }
                }
            },
            None => {
                // Connection not found
                Ok(SocketMessage::new(
                    MSG_TYPE_UNIX_RECEIVE,
                    message.socket_id,
                    message.path,
                    message.ip_addr,
                    message.port,
                    vec![1], // Error: Connection not found
                ).serialize())
            }
        }
    }
    
    fn handle_unix_close(&self, message: SocketMessage) -> Result<Vec<u8>, CmioError> {
        // Find and remove the connection
        let mut connections = self.unix_connections.lock().unwrap();
        let removed = connections.remove(&message.socket_id);
        
        match removed {
            Some(_) => {
                // Return success response
                Ok(SocketMessage::new(
                    MSG_TYPE_UNIX_CLOSE,
                    message.socket_id,
                    message.path,
                    message.ip_addr,
                    message.port,
                    vec![0], // Success
                ).serialize())
            },
            None => {
                // Connection not found
                Ok(SocketMessage::new(
                    MSG_TYPE_UNIX_CLOSE,
                    message.socket_id,
                    message.path,
                    message.ip_addr,
                    message.port,
                    vec![1], // Error: Connection not found
                ).serialize())
            }
        }
    }
    
    fn handle_tcp_connect(&self, message: SocketMessage) -> Result<Vec<u8>, CmioError> {
        // Connect to the TCP socket
        let addr = format!("{}.{}.{}.{}:{}", 
            message.ip_addr[0], message.ip_addr[1], 
            message.ip_addr[2], message.ip_addr[3], 
            message.port);
        let stream = TcpStream::connect(&addr)
            .map_err(|e| CmioError::SetupError(e.raw_os_error().unwrap_or(-1)))?;
        
        // Set non-blocking mode
        stream.set_nonblocking(true)
            .map_err(|e| CmioError::SetupError(e.raw_os_error().unwrap_or(-1)))?;
        
        // Add the connection to our map
        {
            let mut connections = self.tcp_connections.lock().unwrap();
            connections.insert(message.socket_id, (message.path.clone(), stream));
        }
        
        // Return success response
        Ok(SocketMessage::new(
            MSG_TYPE_TCP_CONNECT,
            message.socket_id,
            message.path,
            message.ip_addr,
            message.port,
            vec![0], // Success
        ).serialize())
    }
    
    fn handle_tcp_send(&self, message: SocketMessage) -> Result<Vec<u8>, CmioError> {
        // Find the connection
        let mut connections = self.tcp_connections.lock().unwrap();
        let connection = connections.get_mut(&message.socket_id);
        
        match connection {
            Some((_, stream)) => {
                // Write data to the socket
                stream.write_all(&message.data)
                    .map_err(|e| CmioError::SetupError(e.raw_os_error().unwrap_or(-1)))?;
                
                // Return success response
                Ok(SocketMessage::new(
                    MSG_TYPE_TCP_SEND,
                    message.socket_id,
                    message.path,
                    message.ip_addr,
                    message.port,
                    vec![0], // Success
                ).serialize())
            },
            None => {
                // Connection not found
                Ok(SocketMessage::new(
                    MSG_TYPE_TCP_SEND,
                    message.socket_id,
                    message.path,
                    message.ip_addr,
                    message.port,
                    vec![1], // Error: Connection not found
                ).serialize())
            }
        }
    }
    
    fn handle_tcp_receive(&self, message: SocketMessage) -> Result<Vec<u8>, CmioError> {
        // Find the connection
        let mut connections = self.tcp_connections.lock().unwrap();
        let connection = connections.get_mut(&message.socket_id);
        
        match connection {
            Some((_, stream)) => {
                // Read data from the socket
                let mut buffer = vec![0u8; 4096]; // Read up to 4KB
                match stream.read(&mut buffer) {
                    Ok(n) => {
                        // Return the received data
                        Ok(SocketMessage::new(
                            MSG_TYPE_TCP_RECEIVE,
                            message.socket_id,
                            message.path,
                            message.ip_addr,
                            message.port,
                            buffer[..n].to_vec(),
                        ).serialize())
                    },
                    Err(e) => {
                        if e.kind() == io::ErrorKind::WouldBlock {
                            // No data available
                            Ok(SocketMessage::new(
                                MSG_TYPE_TCP_RECEIVE,
                                message.socket_id,
                                message.path,
                                message.ip_addr,
                                message.port,
                                vec![], // Empty data
                            ).serialize())
                        } else {
                            // Error reading from socket
                            Err(CmioError::SetupError(e.raw_os_error().unwrap_or(-1)))
                        }
                    }
                }
            },
            None => {
                // Connection not found
                Ok(SocketMessage::new(
                    MSG_TYPE_TCP_RECEIVE,
                    message.socket_id,
                    message.path,
                    message.ip_addr,
                    message.port,
                    vec![1], // Error: Connection not found
                ).serialize())
            }
        }
    }
    
    fn handle_tcp_close(&self, message: SocketMessage) -> Result<Vec<u8>, CmioError> {
        // Find and remove the connection
        let mut connections = self.tcp_connections.lock().unwrap();
        let removed = connections.remove(&message.socket_id);
        
        match removed {
            Some(_) => {
                // Return success response
                Ok(SocketMessage::new(
                    MSG_TYPE_TCP_CLOSE,
                    message.socket_id,
                    message.path,
                    message.ip_addr,
                    message.port,
                    vec![0], // Success
                ).serialize())
            },
            None => {
                // Connection not found
                Ok(SocketMessage::new(
                    MSG_TYPE_TCP_CLOSE,
                    message.socket_id,
                    message.path,
                    message.ip_addr,
                    message.port,
                    vec![1], // Error: Connection not found
                ).serialize())
            }
        }
    }
}

#[cfg(all(test, not(target_arch = "riscv64")))]
mod tests {
    use super::*;

    #[test]
    fn test_unix_connect_message() {
        let message = SocketMessage::new(
            MSG_TYPE_UNIX_CONNECT,
            0x12345678,
            "/tmp/test.sock".to_string(),
            [0, 0, 0, 0], // IP not used for Unix connects
            0,            // Port not used for Unix connects
            vec![],      // No data for connect messages
        );

        let serialized = message.serialize();
        let deserialized = SocketMessage::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.msg_type, MSG_TYPE_UNIX_CONNECT);
        assert_eq!(deserialized.socket_id, 0x12345678);
        assert_eq!(deserialized.path, "/tmp/test.sock");
        assert_eq!(deserialized.ip_addr, [0, 0, 0, 0]);
        assert_eq!(deserialized.port, 0);
        assert_eq!(deserialized.data, vec![]);
    }

    #[test]
    fn test_tcp_connect_message() {
        let message = SocketMessage::new(
            MSG_TYPE_TCP_CONNECT,
            0x87654321,
            "".to_string(), // Path not used for TCP connects
            [10, 0, 0, 1],
            443,
            vec![], // No data for connect messages
        );

        let serialized = message.serialize();
        let deserialized = SocketMessage::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.msg_type, MSG_TYPE_TCP_CONNECT);
        assert_eq!(deserialized.socket_id, 0x87654321);
        assert_eq!(deserialized.path, "");
        assert_eq!(deserialized.ip_addr, [10, 0, 0, 1]);
        assert_eq!(deserialized.port, 443);
        assert_eq!(deserialized.data, vec![]);
    }

    #[test]
    fn test_unix_send_message() {
        let message = SocketMessage::new(
            MSG_TYPE_UNIX_SEND,
            0xdeadbeef,
            "".to_string(), // Path not included in non-connect messages
            [0, 0, 0, 0],   // IP not included in non-connect messages
            0,              // Port not included in non-connect messages
            vec![9, 10, 11, 12],
        );

        let serialized = message.serialize();
        let deserialized = SocketMessage::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.msg_type, MSG_TYPE_UNIX_SEND);
        assert_eq!(deserialized.socket_id, 0xdeadbeef);
        assert_eq!(deserialized.path, "");
        assert_eq!(deserialized.ip_addr, [0, 0, 0, 0]);
        assert_eq!(deserialized.port, 0);
        assert_eq!(deserialized.data, vec![9, 10, 11, 12]);
    }

    #[test]
    fn test_tcp_receive_message() {
        let message = SocketMessage::new(
            MSG_TYPE_TCP_RECEIVE,
            0xcafebabe,
            "".to_string(),
            [0, 0, 0, 0],
            0,
            vec![13, 14, 15, 16],
        );

        let serialized = message.serialize();
        let deserialized = SocketMessage::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.msg_type, MSG_TYPE_TCP_RECEIVE);
        assert_eq!(deserialized.socket_id, 0xcafebabe);
        assert_eq!(deserialized.path, "");
        assert_eq!(deserialized.ip_addr, [0, 0, 0, 0]);
        assert_eq!(deserialized.port, 0);
        assert_eq!(deserialized.data, vec![13, 14, 15, 16]);
    }

    #[test]
    fn test_empty_data() {
        let message = SocketMessage::new(
            MSG_TYPE_UNIX_SEND, // Changed from CONNECT to SEND since connects don't have data
            0x12345678,
            "".to_string(),
            [0, 0, 0, 0],
            0,
            vec![], // Empty data for non-connect message
        );

        let serialized = message.serialize();
        let deserialized = SocketMessage::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.data, vec![]);
    }

    #[test]
    fn test_large_data() {
        let large_data = vec![0u8; 1024]; // 1KB of data
        let message = SocketMessage::new(
            MSG_TYPE_TCP_SEND,
            0x12345678,
            "".to_string(),
            [0, 0, 0, 0],
            0,
            large_data.clone(),
        );

        let serialized = message.serialize();
        let deserialized = SocketMessage::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.data, large_data);
    }

    #[test]
    fn test_invalid_message() {
        // Test with insufficient data for basic message
        let invalid_data = vec![0x01, 0x12, 0x34, 0x56, 0x78]; // Only 5 bytes
        assert!(SocketMessage::deserialize(&invalid_data).is_err());

        // Test with insufficient data for Unix connect
        let invalid_unix_connect = vec![
            MSG_TYPE_UNIX_CONNECT,
            0x12, 0x34, 0x56, 0x78,
            5, // path length
            // Missing path data
        ];
        assert!(SocketMessage::deserialize(&invalid_unix_connect).is_err());

        // Test with insufficient data for TCP connect
        let invalid_tcp_connect = vec![
            MSG_TYPE_TCP_CONNECT,
            0x12, 0x34, 0x56, 0x78,
            // Missing IP and port
        ];
        assert!(SocketMessage::deserialize(&invalid_tcp_connect).is_err());

        // Test with invalid message type
        let message = SocketMessage::new(
            0xFF, // Invalid message type
            0x12345678,
            "".to_string(),
            [0, 0, 0, 0],
            0,
            vec![1, 2, 3],
        );

        let serialized = message.serialize();
        let deserialized = SocketMessage::deserialize(&serialized).unwrap();
        assert_eq!(deserialized.msg_type, 0xFF);
    }
} 