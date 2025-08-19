use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::{TcpListener, TcpStream};

use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode, Uri};
use hyper_util::rt::TokioIo;
use http_body_util::{combinators::BoxBody, BodyExt, Empty, Full};
use hyper::body::Bytes;

use clap::Parser;
use base64::{Engine as _, engine::general_purpose};

/// A robust HTTP proxy that requires basic authentication.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value_t = 8080)]
    port: u16,
    #[arg(short, long, required = true)]
    username: String,
    #[arg(long, required = true)]
    password: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    let credentials = Arc::new((args.username, args.password));
    let listener = TcpListener::bind(addr).await?;
    println!("HTTP proxy listening on {}, authentication is required.", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let creds = credentials.clone();
        
        tokio::task::spawn(async move {
            let service = service_fn(move |req| {
                proxy(req, creds.clone())
            });

            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service)
                .with_upgrades()
                .await
            {
                eprintln!("Failed to serve connection: {:?}", err);
            }
        });
    }
}

async fn proxy(
    req: Request<hyper::body::Incoming>,
    credentials: Arc<(String, String)>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    
    // --- Authentication ---
    let (user, pass) = &*credentials;
    if req.headers().get("proxy-authorization").and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
        .and_then(|encoded| general_purpose::STANDARD.decode(encoded).ok())
        .and_then(|decoded| String::from_utf8(decoded).ok())
        .map_or(false, |decoded_str| {
            let mut parts = decoded_str.splitn(2, ':');
            if let (Some(req_user), Some(req_pass)) = (parts.next(), parts.next()) {
                req_user == user && req_pass == pass
            } else {
                false
            }
        })
    == false {
        let mut res = Response::new(full_body("407 Proxy Authentication Required"));
        *res.status_mut() = StatusCode::PROXY_AUTHENTICATION_REQUIRED;
        res.headers_mut().insert("Proxy-Authenticate", "Basic realm=\"Proxy\"".parse().unwrap());
        return Ok(res);
    }

    // --- Request Handling ---
    if Method::CONNECT == req.method() {
        // Handle CONNECT for HTTPS tunneling
        if let Some(addr) = host_addr(req.uri()) {
            tokio::task::spawn(async move {
                match hyper::upgrade::on(req).await {
                    Ok(upgraded) => {
                        if let Err(e) = tunnel(upgraded, addr).await {
                            eprintln!("server io error: {}", e);
                        };
                    }
                    Err(e) => eprintln!("upgrade error: {}", e),
                }
            });
            Ok(Response::new(empty_body()))
        } else {
            eprintln!("CONNECT host is not socket addr: {:?}", req.uri());
            let mut res = Response::new(full_body("CONNECT must be to a socket address"));
            *res.status_mut() = StatusCode::BAD_REQUEST;
            Ok(res)
        }
    } else {
        // Handle HTTP forwarding
        let host = req.uri().host().expect("uri has no host");
        let port = req.uri().port_u16().unwrap_or(80);
        let addr = format!("{}:{}", host, port);
        let stream = TcpStream::connect(addr).await.unwrap();
        let io = TokioIo::new(stream);

        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
        tokio::task::spawn(async move {
            if let Err(err) = conn.await {
                println!("Connection failed: {:?}", err);
            }
        });

        let res = sender.send_request(req).await?;
        Ok(res.map(|b| b.boxed()))
    }
}

fn host_addr(uri: &Uri) -> Option<String> {
    uri.authority().map(|auth| auth.to_string())
}

fn empty_body() -> BoxBody<Bytes, hyper::Error> {
    Empty::<Bytes>::new().map_err(|e| match e {}).boxed()
}

fn full_body(chunk: &'static str) -> BoxBody<Bytes, hyper::Error> {
    Full::new(Bytes::from(chunk)).map_err(|e| match e {}).boxed()
}

async fn tunnel(upgraded: hyper::upgrade::Upgraded, addr: String) -> std::io::Result<()> {
    let mut server = TcpStream::connect(addr).await?;
    let mut upgraded = TokioIo::new(upgraded);
    tokio::io::copy_bidirectional(&mut upgraded, &mut server).await?;
    Ok(())
}