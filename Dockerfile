# Stage 1: Build Environment (Using rust:bookworm matches our debian:bookworm-slim runtime)
FROM rust:bookworm AS builder

WORKDIR /usr/src/app

# Copy the manifests and source code
COPY Cargo.toml ./
# COPY Cargo.lock ./ # Uncomment if you have a lock file
COPY src ./src
COPY configs ./configs

# Build the release binaries
RUN cargo build --release

# Stage 2: Runtime Environment
FROM ubuntu:26.04

# Install ca-certificates and bash
RUN apt-get update && apt-get install -y ca-certificates tzdata bash gcc clang && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Expose port 3000
EXPOSE 3000

# Create the configs directory so the bot can read/write state
RUN mkdir -p /app/configs

# Copy BOTH compiled binaries from the builder stage
COPY --from=builder /usr/src/app/target/release/cyrus-bot /usr/local/bin/cyrus_bot
COPY --from=builder /usr/src/app/target/release/cyrus-api /usr/local/bin/cyrus_api

# Create a startup script to run both
# We start the API in the background (&), then use `exec` for the bot so it becomes PID 1
RUN echo '#!/bin/bash\n\
echo "Starting Cyrus API..."\n\
/usr/local/bin/cyrus_api &\n\
echo "Starting Cyrus Bot..."\n\
exec /usr/local/bin/cyrus_bot\n\
' > /usr/local/bin/start.sh && chmod +x /usr/local/bin/start.sh

# Set up standard logging
ENV RUST_LOG=info

# Run the startup script
CMD ["/usr/local/bin/start.sh"]