FROM harbor.ocean.io/public/messensemanylinux_2_28-cross:${TARGETARCH} AS color_palette_extract
RUN curl https://sh.rustup.rs -sSf | bash -s -- -y

ENV PATH="/root/.cargo/bin:${PATH}"
RUN pip install maturin==1.1.0

WORKDIR /usr/rustbuild

# COPY backend-rust/color_palette_extract/Cargo.lock ./Cargo.lock
# COPY backend-rust/color_palette_extract/Cargo.toml ./Cargo.toml
# RUN mkdir -p src && touch src/lib.rs && cargo build --release --locked

ADD backend-rust/color_palette_extract/ .

RUN maturin build --release --strip -i python3.12
    

RUN mv target/wheels/*.whl ./
    
