use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use clap::Parser;
use base64::{Engine as _, engine::general_purpose};

/// A simple HTTP proxy that supports basic authentication and CONNECT method.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value_t = 8080)]
    port: u16,

    /// Username for authentication
    #[arg(short, long)]
    username: Option<String>,

    /// Password for authentication
    #[arg(long)]
    password: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    if args.username.is_some() && args.password.is_none() {
        return Err("Password is required when username is provided.".into());
    }

    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    let listener = TcpListener::bind(addr).await?;
    println!("HTTP proxy listening on {}", addr);

    let credentials = Arc::new((args.username, args.password));

    loop {
        let (stream, _) = listener.accept().await?;
        let creds = credentials.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, creds).await {
                eprintln!("failed to handle connection: {}", e);
            }
        });
    }
}

async fn handle_connection(mut stream: TcpStream, credentials: Arc<(Option<String>, Option<String>)>) -> Result<(), Box<dyn std::error::Error>> {
    let mut buffer = [0; 4096];
    let n = stream.read(&mut buffer).await?;
    let request_str = String::from_utf8_lossy(&buffer[..n]);

    let (username, password) = &*credentials;
    if let (Some(user), Some(pass)) = (username, password) {
        let auth_header = request_str.lines()
            .find(|line| line.to_lowercase().starts_with("proxy-authorization:"));

        let mut authenticated = false;
        if let Some(header) = auth_header {
            if let Some(encoded) = header.split_whitespace().nth(1) {
                if let Ok(decoded_bytes) = general_purpose::STANDARD.decode(encoded) {
                    if let Ok(decoded_str) = String::from_utf8(decoded_bytes) {
                        let mut parts = decoded_str.splitn(2, ':');
                        if let (Some(req_user), Some(req_pass)) = (parts.next(), parts.next()) {
                            if req_user == user && req_pass == pass {
                                authenticated = true;
                            }
                        }
                    }
                }
            }
        }

        if !authenticated {
            let response = "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"Proxy\"\r\n\r\n";
            stream.write_all(response.as_bytes()).await?;
            return Ok(());
        }
    }

    let mut lines = request_str.lines();
    let first_line = lines.next().ok_or("Empty request")?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next().ok_or("Invalid method")?;
    let target = parts.next().ok_or("Invalid target")?;

    if method.eq_ignore_ascii_case("CONNECT") {
        let response = "HTTP/1.1 200 Connection Established\r\n\r\n";
        stream.write_all(response.as_bytes()).await?;

        let mut server_stream = TcpStream::connect(target).await?;
        tokio::io::copy_bidirectional(&mut stream, &mut server_stream).await?;
    } else {
        let url = url::Url::parse(target)?;
        let host = url.host_str().ok_or("Invalid host in URL")?;
        let port = url.port().unwrap_or(80);

        let server_addr = format!("{}:{}", host, port);
        let mut server_stream = TcpStream::connect(server_addr).await?;

        let path = url.path();
        let query = url.query();
        let mut new_first_line = format!("{} {}", method, path);
        if let Some(q) = query {
            new_first_line.push('?');
            new_first_line.push_str(q);
        }
        new_first_line.push_str(" HTTP/1.1\r\n");

        let mut new_request = new_first_line.to_string();
        new_request.push_str(&format!("Host: {}\r\n", host));
        for line in lines {
            if !line.to_lowercase().starts_with("proxy-authorization:") && !line.to_lowercase().starts_with("host:") {
                new_request.push_str(line);
                new_request.push_str("\r\n");
            }
        }
        new_request.push_str("\r\n");

        server_stream.write_all(new_request.as_bytes()).await?;
        tokio::io::copy_bidirectional(&mut stream, &mut server_stream).await?;
    }

    Ok(())
}