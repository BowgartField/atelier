use jean_core::server::{spawn, HeadlessDispatcher, ServerConfig};
use jean_core::{BackendContext, BackendState, HeadlessAppPaths, ServerEventSink, WsBroadcaster};
use std::path::PathBuf;
use std::sync::Arc;

const APP_IDENTIFIER: &str = "com.jean.desktop";

#[derive(Debug)]
struct CliArgs {
    host: String,
    port: u16,
    token: String,
    token_required: bool,
    data_dir: Option<PathBuf>,
}

impl CliArgs {
    fn parse() -> Result<Self, String> {
        let mut host = std::env::var("JEAN_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let mut port = std::env::var("JEAN_PORT")
            .ok()
            .map(|value| value.parse::<u16>().map_err(|error| error.to_string()))
            .transpose()?
            .unwrap_or(3456);
        let mut token = std::env::var("JEAN_TOKEN").unwrap_or_default();
        let mut token_required = std::env::var("JEAN_NO_TOKEN").as_deref() != Ok("1");
        let mut data_dir = std::env::var_os("JEAN_DATA_DIR").map(PathBuf::from);
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--host" => host = args.next().ok_or("--host requires a value")?,
                "--port" => {
                    port = args
                        .next()
                        .ok_or("--port requires a value")?
                        .parse::<u16>()
                        .map_err(|error| format!("Invalid --port: {error}"))?;
                }
                "--token" => token = args.next().ok_or("--token requires a value")?,
                "--no-token" => token_required = false,
                "--data-dir" => {
                    data_dir = Some(PathBuf::from(
                        args.next().ok_or("--data-dir requires a value")?,
                    ));
                }
                "--help" | "-h" => {
                    println!(
                        "jean-server [--host IP] [--port PORT] [--token TOKEN] [--no-token] [--data-dir PATH]"
                    );
                    std::process::exit(0);
                }
                unknown => return Err(format!("Unknown argument: {unknown}")),
            }
        }
        if token_required && token.is_empty() {
            token = jean_core::auth::generate_token();
        }
        Ok(Self {
            host,
            port,
            token,
            token_required,
            data_dir,
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = CliArgs::parse().map_err(|error| format!("Invalid arguments: {error}"))?;
    let paths = Arc::new(HeadlessAppPaths::resolve_with_data_dir(
        APP_IDENTIFIER,
        args.data_dir,
    )?);
    paths.ensure_directories()?;
    let broadcaster = Arc::new(WsBroadcaster::new());
    let state = Arc::new(BackendState::new(broadcaster.clone()));
    let events = Arc::new(ServerEventSink::new(broadcaster));
    let context = BackendContext::new(paths, events, state);
    let dispatcher = Arc::new(HeadlessDispatcher::new(context.clone()));
    let handle = spawn(
        context,
        ServerConfig {
            host: args.host,
            port: args.port,
            token: args.token.clone(),
            token_required: args.token_required,
            allowed_origins: jean_core::server::parse_allowed_origins(
                &std::env::var("JEAN_ALLOWED_ORIGINS").unwrap_or_default(),
            ),
        },
        dispatcher,
    )
    .await?;

    println!("Jean server listening on http://{}", handle.address);
    if args.token_required {
        println!("Authentication token: {}", args.token);
    }
    wait_for_shutdown_signal().await?;
    handle.shutdown().await?;
    Ok(())
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() -> Result<(), std::io::Error> {
    use tokio::signal::unix::{signal, SignalKind};
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() -> Result<(), std::io::Error> {
    tokio::signal::ctrl_c().await
}
