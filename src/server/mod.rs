use std::convert::Infallible;
use std::net::SocketAddr;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Bytes;
use hyper::{Method, Request, Response, StatusCode};
use hyper::server::conn::http1::Builder;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use log::info;
use tokio::net::TcpListener;


pub const PORT: u16 = 20451;

pub async fn run() {
    let addr = SocketAddr::from(([127, 0, 0, 1], PORT));

    let listener = TcpListener::bind(addr).await.expect("Unable to bind to address");

    info!("streamgui listening on: http://{}", addr);

    loop {
        match listener.accept().await {
            Ok((socket, addr)) => {
                info!("accepted connection from {}", addr);

                let io = TokioIo::new(socket);

                tokio::task::spawn(async move {
                    if let Err(err) = Builder::new()
                        .timer(TokioTimer::default())
                        .serve_connection(io, service_fn(http_server_handler))
                        .await
                    {
                        info!("http error: {}", err);
                    }
                });

            },
            Err(e) => {
                info!("failed to accept connection: {}", e);
            }
        }
    }
}

async fn http_server_handler(req: Request<hyper::body::Incoming>) -> Result<Response<BoxBody<Bytes, hyper::Error>>, Infallible> {

    match (req.method(), req.uri().path()) {
        // todo have some js do fancy things
        (&Method::GET, "/") => Ok(Response::new(full("Copy the token out of the URL above!"))),

        // Return the 404 Not Found for other routes.
        _ => {
            let mut not_found = Response::new(empty());
            *not_found.status_mut() = StatusCode::NOT_FOUND;
            Ok(not_found)
        }
    }
}


fn empty() -> BoxBody<Bytes, hyper::Error> {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
}

fn full<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}
