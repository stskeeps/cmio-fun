# TAP CMIO Rust Interface

This project provides a Rust interface for interacting with the Machine I/O (CMIO) system. It implements the core functionality for communicating with the machine through the CMIO device.

## Features

- CMIO device initialization and setup
- Memory mapping for TX and RX buffers
- Yield operation support
- Safe Rust abstractions over low-level system calls
- TAP network interface integration for network communication
- Unix domain socket support for inter-process communication
- Optimized data batching for improved throughput

## Building

### Local Development

```bash
cargo build
```

### Cross-compilation to RISC-V

The project includes a Dockerfile for cross-compilation to RISC-V:

```bash
docker build -t tapcmio .
```

## Usage

### Command Line Options

The program supports different modes of operation:

```bash
# Run in network mode (TAP interface)
cargo run -- network

# Run in Unix domain socket mode
cargo run -- unix

# Show help
cargo run -- help
```

### Basic CMIO Usage

```rust
use tapcmio::{Cmio, CmioYield};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmio = Cmio::new()?;
    
    let mut yield_data = CmioYield {
        dev: 0,
        cmd: 0,
        reason: 0,
        data: 0,
    };
    
    cmio.yield_(&mut yield_data)?;
    Ok(())
}
```

### Using the Convenience Function

```rust
use tapcmio::{Cmio, CmioYield};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmio = Cmio::new()?;
    
    let tx_data = b"Hello, TAP CMIO!";
    let (rx_data, reason) = cmio.yield_with_buffer(1, 2, 3, tx_data)?;
    
    println!("Received {} bytes with reason {}", rx_data.len(), reason);
    Ok(())
}
```

### Network Interface

The project includes a TAP network interface that can be used to communicate with the machine over the network:

```rust
use tapcmio::network::NetworkInterface;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut network = NetworkInterface::new()?;
    network.run_loop()?;
    Ok(())
}
```

The network interface implements an optimized loop for maximum throughput:
1. Check if there's data to transmit from the TAP interface (batching multiple packets)
2. If yes, send the batched data via CMIO yield
3. If no, send a zero-length yield to check for incoming data
4. Process any received data by writing it to the TAP interface
5. If no data to transmit or receive, yield to the scheduler
6. Repeat

### Unix Domain Socket Interface

The project also includes a Unix domain socket interface for inter-process communication:

```rust
use tapcmio::unix_socket::UnixSocketManager;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cmio = Cmio::new()?;
    let cmio_max_buffer_size = cmio.get_tx_length();
    let unix_socket = UnixSocketManager::new(cmio, cmio_max_buffer_size);
    unix_socket.run_loop()?;
    Ok(())
}
```

The Unix domain socket interface supports the following operations:
1. **Connect**: Establish a connection to a Unix domain socket
2. **Send**: Send data to a connected socket
3. **Receive**: Receive data from a connected socket
4. **Close**: Close a connection

Multiple messages can be sent and received in a single CMIO transmission, improving throughput.

#### Message Format

Each message has the following format:
- Message type (1 byte)
- Path length (1 byte)
- Path (variable length)
- Data length (4 bytes, network byte order)
- Data (variable length)

#### Performance Optimizations

The Unix domain socket interface includes several optimizations:
- **Message Batching**: Multiple messages are processed in a single CMIO transmission
- **Connection Management**: Connections are maintained for reuse
- **Efficient Scheduling**: Only yields to the scheduler when there's no data to process
- **Non-blocking I/O**: Uses non-blocking reads to efficiently handle socket traffic

## Error Handling

The library provides detailed error types through the `CmioError` enum:

- `OpenError`: Failed to open the CMIO device
- `SetupError`: Failed to setup CMIO
- `MapError`: Failed to map memory
- `BufferTooLarge`: Buffer size exceeds the maximum allowed size

## License

This project is licensed under the Apache License 2.0 - see the LICENSE file for details.
