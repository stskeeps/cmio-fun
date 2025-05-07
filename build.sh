#!/bin/bash
set -e

# Build the Docker image
docker build -t tap-cmio .

# Extract the binary from the Docker image
docker create --name temp-container tap-cmio
docker cp temp-container:/usr/local/bin/tapcmio ./tapcmio-riscv64
docker rm temp-container

echo "Binary extracted to ./tapcmio-riscv64" 