FROM ubuntu:24.04 as builder

# Install RISC-V target and cross-compilation tools
RUN apt-get update && apt-get install -y \
    build-essential binutils-riscv64-linux-gnu \
    ca-certificates curl \
    gcc-riscv64-linux-gnu \
    g++-riscv64-linux-gnu \
    curl \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH \
    RUST_VERSION=1.86.0

    RUN <<EOF
set -eux
dpkgArch="$(dpkg --print-architecture)"
case "${dpkgArch##*-}" in \
    amd64) rustArch='x86_64-unknown-linux-gnu'; rustupSha256='6aeece6993e902708983b209d04c0d1dbb14ebb405ddb87def578d41f920f56d' ;;
    armhf) rustArch='armv7-unknown-linux-gnueabihf'; rustupSha256='3c4114923305f1cd3b96ce3454e9e549ad4aa7c07c03aec73d1a785e98388bed' ;;
    arm64) rustArch='aarch64-unknown-linux-gnu'; rustupSha256='1cffbf51e63e634c746f741de50649bbbcbd9dbe1de363c9ecef64e278dba2b2' ;;
    i386) rustArch='i686-unknown-linux-gnu'; rustupSha256='0a6bed6e9f21192a51f83977716466895706059afb880500ff1d0e751ada5237' ;;
    *) echo >&2 "unsupported architecture: ${dpkgArch}"; exit 1 ;;
esac
url="https://static.rust-lang.org/rustup/archive/1.27.1/${rustArch}/rustup-init"
curl -fsSL -O "$url"
echo "${rustupSha256} *rustup-init" | sha256sum -c -
chmod +x rustup-init
./rustup-init -y --no-modify-path --profile minimal --default-toolchain $RUST_VERSION --default-host ${rustArch}
rm rustup-init
chmod -R a+w $RUSTUP_HOME $CARGO_HOME
rustup --version
cargo --version
rustc --version
EOF

RUN rustup target add riscv64gc-unknown-linux-gnu

WORKDIR /usr/src/app
COPY . .
RUN ls -lR

# Build for RISC-V
RUN cargo build --target riscv64gc-unknown-linux-gnu --release

# Create a minimal runtime image
FROM --platform=linux/riscv64 ubuntu:24.04

COPY --from=builder /usr/src/app/target/riscv64gc-unknown-linux-gnu/release/tapcmio /usr/local/bin/

CMD ["tapcmio"] 