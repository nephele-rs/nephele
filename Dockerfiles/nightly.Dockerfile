# Build the binary
FROM rustlang/rust:nightly as builder

# Copy in the cargo src
WORKDIR /nephele
COPY ../nephele/*             /nephele

# Build
RUN cargo build --example h1_server --release

#FROM alpine:3.13
FROM frolvlad/alpine-glibc

RUN echo "https://mirrors.aliyun.com/alpine/v3.13/main/" > /etc/apk/repositories
RUN echo "https://mirrors.aliyun.com/alpine/v3.13/community" >> /etc/apk/repositories

RUN apk add --no-cache \
    ca-certificates \
    gcc; \
    apk add iptables

RUN apk add bash bash-doc bash-completion

WORKDIR /nephele
COPY --from=builder /nephele/target/release/examples/h1_server  /nephele/bin/
COPY examples/identity.pfx /nephele/bin/
RUN chmod +x /nephele/bin/h1_server

USER root
CMD ["/nephele/bin/h1_server"]
