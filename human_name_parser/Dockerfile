FROM harbor.ocean.io/public/messensemanylinux_2_28-cross:${TARGETARCH} AS human_name_parser
# RUN apt install -y python3-pip python3
RUN curl https://sh.rustup.rs -sSf | bash -s -- -y

ENV PATH="/root/.cargo/bin:${PATH}"

RUN pip install maturin==1.1.0


WORKDIR /usr/rustbuild

# COPY backend-rust/human_name_parser/Cargo.lock ./Cargo.lock
# COPY backend-rust/human_name_parser/Cargo.toml ./Cargo.toml
# RUN mkdir -p src && touch src/lib.rs && cargo build --release --locked

ADD backend-rust/human_name_parser/ .

RUN maturin build --release --strip -i python3.12

# RUN ls target/wheels
# RUN uname -m
RUN mv target/wheels/*.whl ./
