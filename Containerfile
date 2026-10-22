# Cubby image. Debian release pinned in both FROM lines (builder glibc <= runtime glibc).
FROM docker.io/library/node:22-bookworm-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM docker.io/library/rust:1.99.1-slim-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
RUN cargo build --release -p cubby-server -p cubby-worker

FROM docker.io/library/debian:bookworm-slim
COPY --from=builder /app/target/release/cubby-server /app/target/release/cubby-worker /usr/local/bin/
COPY --from=web /web/build /srv/cubby/web
ENV CUBBY_BIND=0.0.0.0 CUBBY_PORT=8790 CUBBY_WEB_DIR=/srv/cubby/web
EXPOSE 8790
CMD ["cubby-server"]
