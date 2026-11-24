use hyper::server::conn::http1;
use hyper::service::Service as HyperService;
use hyper_util::rt::TokioIo;
use lanlord::config;
use lanlord::db::Db;
use lanlord::discovery::ArpDiscovery;
use lanlord::web::{self, AppState};
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tower::Service as TowerService;

type AppRouter = axum::Router<()>;
type AppRequest = http::Request<hyper::body::Incoming>;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("lanlord: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = config::parse_cli()?;
    let config = config::load(&cli)?;
    config::validate(&config)?;

    let db = Arc::new(Db::open(&config.database)?);
    let state = Arc::new(AppState {
        db,
        config,
        discovery: Arc::new(ArpDiscovery),
    });

    // `with_state` provides the router its state and turns
    // `Router<Arc<AppState>>` into `Router<()>`, the only router type that is
    // a `Service` and can be served directly.
    let listen = state.config.listen.clone();
    let app: AppRouter = web::router().with_state(state);

    let listener = tokio::net::TcpListener::bind(&listen).await?;
    eprintln!("lanlord: listening on {listen}");

    loop {
        let (stream, _addr) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let svc = RequestService {
            router: app.clone(),
        };
        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new().serve_connection(io, svc).await {
                eprintln!("lanlord: connection error: {err}");
            }
        });
    }
}

/// Handles a single HTTP request through the axum router, which already holds
/// the shared state.
#[derive(Clone, Debug)]
struct RequestService {
    router: AppRouter,
}

impl HyperService<AppRequest> for RequestService {
    type Response = axum::response::Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: AppRequest) -> Self::Future {
        let mut router = self.router.clone();
        Box::pin(async move {
            let (parts, incoming) = req.into_parts();
            let body = axum::body::Body::new(incoming);
            let req = http::Request::from_parts(parts, body);
            match TowerService::call(&mut router, req).await {
                Ok(response) => Ok(response),
                Err(infallible) => match infallible {},
            }
        })
    }
}
