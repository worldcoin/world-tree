### Base stage
FROM debian:12 as base

WORKDIR /src

# Install dependencies
RUN apt-get update && \
  apt-get install -y \
  curl build-essential \
  libssl-dev texinfo \
  libcap2-bin pkg-config

COPY ./rust-toolchain.toml ./rust-toolchain.toml

# Install rustup
RUN curl https://sh.rustup.rs -sSf | sh -s -- -y --default-toolchain 1.81.0

# Set environment variables
ENV PATH="/root/.cargo/bin:${PATH}"
ENV RUSTUP_HOME="/root/.rustup"
ENV CARGO_HOME="/root/.cargo"

# Install the toolchain
RUN rustup component add cargo

# Install cargo chef
RUN cargo install cargo-chef --locked

### Recipe cooking stage
FROM base as build-env-base
WORKDIR /src

# Copy everything
COPY . .

# Prepare the recipe
RUN cargo chef prepare --recipe-path recipe.json

### Build stage
FROM base as build-env
WORKDIR /src

# Copy recipe
COPY --from=build-env-base /src/recipe.json ./recipe.json


# Copy just the crates
COPY crates/ crates/

# Build the dependencies
RUN cargo chef cook --release --recipe-path ./recipe.json

# Copy the remaining source code
COPY . .

ARG BIN=world-tree

# Build the binary
RUN cargo build --release --bin $BIN --no-default-features

# Make sure it runs
RUN /src/target/release/$BIN --version

### Runtime stage
## cc variant because we need libgcc and others
FROM gcr.io/distroless/cc-debian12:nonroot

ARG BIN=world-tree

# Copy the binary
COPY --from=build-env --chown=0:10001 --chmod=454 /src/target/release/$BIN /bin/app

ENTRYPOINT [ "/bin/app" ]
