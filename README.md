# My HTTP Proxy

A simple asynchronous HTTP proxy written in Rust.

## Features

- Custom port selection
- Optional basic authentication (username/password)
- Supports `CONNECT` for HTTPS traffic
- Forwards standard HTTP traffic

## Build

To build the proxy, run the following command:

```sh
cargo build --release
```

The executable will be located at `target/release/mypproxy`.

## Usage

### Run without authentication

```sh
./target/release/mypproxy --port 8080
```

### Run with authentication

```sh
./target/release/mypproxy --port 8080 --username myuser --password mypass
```

### Using with `curl`

#### HTTPS traffic (CONNECT)

```sh
# Without authentication
curl -x http://127.0.0.1:8080 https://www.rust-lang.org/

# With authentication
curl -x http://127.0.0.1:8080 -U myuser:mypass https://www.rust-lang.org/
```

#### HTTP traffic

```sh
# Without authentication
curl -x http://127.0.0.1:8080 http://httpbin.org/get

# With authentication
curl -x http://127.0.0.1:8080 -U myuser:mypass http://httpbin.org/get
```
