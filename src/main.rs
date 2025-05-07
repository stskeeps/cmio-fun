mod cmio;
mod network;
mod unix_tcp_socket;

use std::env;
use cmio::{Cmio, CmioYield};
use network::NetworkInterface;
use unix_tcp_socket::SocketManager;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("TAP CMIO Interface");
    println!("==================");
    
    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    let mode = if args.len() > 1 {
        args[1].as_str()
    } else {
        "help"
    };
    
    match mode {
        "network" => run_network_mode()?,
        "unix" => run_unix_socket_mode()?,
        "help" | _ => {
            println!("Usage: {} [mode]", args[0]);
            println!("Modes:");
            println!("  network  - Run in network mode (TAP interface)");
            println!("  unix     - Run in Unix domain socket mode");
            println!("  help     - Show this help message");
        }
    }
    
    Ok(())
}

fn run_network_mode() -> Result<(), Box<dyn std::error::Error>> {
    println!("Running in network mode");
    
    // Example 1: Basic CMIO functionality
    println!("\nTesting basic CMIO functionality...");
    let mut cmio = Cmio::new()?;
    println!("CMIO initialized successfully");

    let mut yield_data = CmioYield {
        dev: 0,
        cmd: 0,
        reason: 0,
        data: 0,
    };

    println!("Performing basic yield operation...");
    cmio.yield_(&mut yield_data)?;
    println!("Yield completed with: dev={}, cmd={}, reason={}, data={}",
        yield_data.dev, yield_data.cmd, yield_data.reason, yield_data.data);

    // Example 2: Using the convenience function with a buffer
    println!("\nTesting yield with buffer...");
    let tx_data = b"Hello, TAP CMIO!";
    let (rx_data, reason) = cmio.yield_with_buffer(1, 2, 3, tx_data)?;
    
    println!("Sent {} bytes: {:?}", tx_data.len(), tx_data);
    println!("Received {} bytes with reason {}: {:?}", rx_data.len(), reason, rx_data);

    // Example 3: Network interface
    println!("\nInitializing network interface...");
    let mut network = NetworkInterface::new()?;
    println!("Network interface initialized successfully");
    
    // Run the network interface loop
    println!("\nStarting network interface loop (press Ctrl+C to exit)...");
    network.run_loop()?;

    Ok(())
}

fn run_unix_socket_mode() -> Result<(), Box<dyn std::error::Error>> {
    println!("Running in Unix domain socket mode");
    
    // Initialize CMIO
    println!("\nInitializing CMIO...");
    let cmio = Cmio::new()?;
    println!("CMIO initialized successfully");
    
    // Get the CMIO max buffer size
    let cmio_max_buffer_size = cmio.get_tx_length();
    println!("CMIO max buffer size: {} bytes", cmio_max_buffer_size);
    
    // Initialize socket manager
    println!("\nInitializing socket manager...");
    let socket_manager = SocketManager::new(cmio, cmio_max_buffer_size);
    println!("Socket manager initialized successfully");
    
    // Run the socket manager loop
    println!("\nStarting socket manager loop (press Ctrl+C to exit)...");
    socket_manager.run_loop()?;
    
    Ok(())
}
