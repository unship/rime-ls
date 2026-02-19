use rime_ls::config::{load_from_file, server_config_path};
use rime_ls::lsp::Backend;
use rime_ls::logger;
use rime_ls::rime::Rime;
use rime_ls::utils;
use std::path::PathBuf;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::broadcast::{self, Receiver};
use tower_lsp::{LspService, Server};

async fn run_stdio() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::build(Backend::new).finish();
    Server::new(stdin, stdout, socket).serve(service).await;
}

async fn run_stream<R, W>(read: R, write: W)
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (service, socket) = LspService::build(Backend::new).finish();
    Server::new(read, write, socket).serve(service).await;
}

/// Daemon mode: runs forever, client disconnect does not affect the server.
async fn run_tcp_forever(bind_addr: std::net::SocketAddr) -> tokio::io::Result<()> {
    use tokio::net::TcpListener;

    logger::info(format!("Listening on TCP: {}", &bind_addr));
    let listener = TcpListener::bind(bind_addr).await?;
    loop {
        let (stream, _) = listener.accept().await?;
        let (read, write) = tokio::io::split(stream);
        tokio::spawn(run_stream(read, write));
    }
}

/// Daemon mode: runs forever, client disconnect does not affect the server.
#[cfg(unix)]
async fn run_unix_forever(socket_path: PathBuf) -> tokio::io::Result<()> {
    use tokio::net::UnixListener;

    // Remove stale socket file if exists
    let _ = std::fs::remove_file(&socket_path);

    logger::info(format!("Listening on Unix socket: {}", socket_path.display()));
    let listener = UnixListener::bind(&socket_path)?;

    loop {
        let (stream, _) = listener.accept().await?;
        let (read, write) = tokio::io::split(stream);
        tokio::spawn(run_stream(read, write));
    }
}

#[cfg(unix)]
fn default_unix_socket_path() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".local/run"))
                .unwrap_or_else(|_| PathBuf::from("/tmp"))
        })
        .join("rime-ls.sock")
}

/// Treat connection-closed errors as normal (not fatal).
#[cfg(unix)]
fn is_connection_closed(err: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    matches!(
        err.kind(),
        ErrorKind::BrokenPipe | ErrorKind::ConnectionReset | ErrorKind::UnexpectedEof
    )
}

/// Proxy: forward LSP messages between editor (stdio) and daemon (socket).
/// When stdin closes first (editor exits), continue forwarding socket→stdout until daemon closes,
/// so shutdown/exit sequence completes and Helix doesn't see "server closed the stream".
#[cfg(unix)]
async fn run_proxy(stream: tokio::net::UnixStream) -> tokio::io::Result<()> {
    let (mut socket_read, mut socket_write) = tokio::io::split(stream);
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();

    let client_to_server = tokio::spawn(async move {
        let r = tokio::io::copy(&mut stdin, &mut socket_write).await;
        drop(socket_write);
        r
    });
    let server_to_client =
        tokio::spawn(async move { tokio::io::copy(&mut socket_read, &mut stdout).await });

    let (r1, r2) = tokio::join!(client_to_server, server_to_client);

    for r in [r1, r2] {
        match r {
            Ok(Ok(_)) => {}
            Ok(Err(e)) if is_connection_closed(&e) => {}
            Ok(Err(e)) => return Err(e),
            Err(join_err) => return Err(tokio::io::Error::new(
                tokio::io::ErrorKind::Other,
                join_err.to_string(),
            )),
        }
    }
    Ok(())
}

#[cfg(unix)]
async fn run_connect(socket_path: PathBuf, mut shutdown: Receiver<()>) -> tokio::io::Result<()> {
    use std::process::{Command, Stdio};
    use tokio::net::UnixStream;
    use tokio::time::{sleep, Duration};

    const CONNECT_RETRIES: u32 = 10;
    const RETRY_DELAY_MS: u64 = 200;
    const STALE_LOCK_RETRIES: u32 = 3;

    async fn try_connect(path: &PathBuf) -> tokio::io::Result<UnixStream> {
        UnixStream::connect(path).await
    }

    for _ in 0..STALE_LOCK_RETRIES {
        // Try connect first
        if let Ok(stream) = try_connect(&socket_path).await {
            tokio::select! {
                _ = shutdown.recv() => return Ok(()),
                _ = run_proxy(stream) => ()
            }
            return Ok(());
        }

        // Server not running - try to start it
        let lock_path = socket_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("/tmp"))
            .join("rime-ls.lock");

        let lock_created = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .is_ok();

        if lock_created {
        // Create parent dir for socket if needed
        if let Some(parent) = socket_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let exe = std::env::current_exe().unwrap_or_else(|_| "rime_ls".into());
        let path_str = socket_path.to_string_lossy().to_string();

        let child = Command::new(&exe)
            .args(["--listen-unix", &path_str])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| tokio::io::Error::new(tokio::io::ErrorKind::Other, e))?;

        // Don't wait for child - let it run in background
        std::mem::forget(child);

        // Wait for server to be ready
        for _ in 0..CONNECT_RETRIES {
            tokio::select! {
                _ = shutdown.recv() => return Ok(()),
                _ = sleep(Duration::from_millis(RETRY_DELAY_MS)) => ()
            }
            if let Ok(stream) = try_connect(&socket_path).await {
                let _ = std::fs::remove_file(&lock_path);
                tokio::select! {
                    _ = shutdown.recv() => return Ok(()),
                    _ = run_proxy(stream) => ()
                }
                return Ok(());
            }
        }

        let _ = std::fs::remove_file(&lock_path);
        return Err(tokio::io::Error::new(
            tokio::io::ErrorKind::ConnectionRefused,
            "failed to start rime-ls server",
        ));
    }

    // Another process has the lock - wait for it to start the server
    for _ in 0..CONNECT_RETRIES {
        tokio::select! {
            _ = shutdown.recv() => return Ok(()),
            _ = sleep(Duration::from_millis(RETRY_DELAY_MS)) => ()
        }
        if let Ok(stream) = try_connect(&socket_path).await {
            tokio::select! {
                _ = shutdown.recv() => return Ok(()),
                _ = run_proxy(stream) => ()
            }
            return Ok(());
        }
    }

    // Lock exists but socket never appeared - likely stale lock from crashed server
    if !socket_path.exists() {
        let _ = std::fs::remove_file(&lock_path);
        tokio::select! {
            _ = shutdown.recv() => return Ok(()),
            _ = sleep(Duration::from_millis(RETRY_DELAY_MS)) => ()
        }
        continue;
    }

    return Err(tokio::io::Error::new(
        tokio::io::ErrorKind::ConnectionRefused,
        "rime-ls server not available",
    ));
    }

    Err(tokio::io::Error::new(
        tokio::io::ErrorKind::ConnectionRefused,
        "rime-ls server not available",
    ))
}

fn run_deploy() -> Result<(), Box<dyn std::error::Error>> {
    let config = load_from_file(server_config_path());
    let shared = utils::expand_tilde(&config.shared_data_dir);
    let user = utils::expand_tilde(&config.user_data_dir);
    let log = utils::expand_tilde(&config.log_dir);
    let (shared, user, log) = (
        shared.to_str().ok_or("invalid shared_data_dir")?.to_string(),
        user.to_str().ok_or("invalid user_data_dir")?.to_string(),
        log.to_str().ok_or("invalid log_dir")?.to_string(),
    );
    init_logger();
    Rime::init(&shared, &user, &log)?;
    Rime::global().deploy();
    Rime::global().destroy();
    println!("Rime deployed successfully.");
    Ok(())
}

fn run_sync() -> Result<(), Box<dyn std::error::Error>> {
    let config = load_from_file(server_config_path());
    let shared = utils::expand_tilde(&config.shared_data_dir);
    let user = utils::expand_tilde(&config.user_data_dir);
    let log = utils::expand_tilde(&config.log_dir);
    let (shared, user, log) = (
        shared.to_str().ok_or("invalid shared_data_dir")?.to_string(),
        user.to_str().ok_or("invalid user_data_dir")?.to_string(),
        log.to_str().ok_or("invalid log_dir")?.to_string(),
    );
    init_logger();
    Rime::init(&shared, &user, &log)?;
    Rime::global().sync_user_data();
    Rime::global().destroy();
    println!("Rime user data synced successfully.");
    Ok(())
}

async fn run(mut shutdown: Receiver<()>) -> Result<(), Box<dyn std::error::Error>> {
    use std::net::SocketAddr;
    use std::str::FromStr;

    let mut args = std::env::args();
    match args.nth(1).as_deref() {
        None => {
            tokio::select! {
                _ = shutdown.recv() => (),
                _ = run_stdio() => ()
            }
        }
        Some("--listen") => {
            init_logger();
            let addr = args.next().unwrap_or_else(|| "127.0.0.1:9257".to_owned());
            let addr = SocketAddr::from_str(&addr)?;
            tokio::select! {
                _ = shutdown.recv() => Ok::<(), Box<dyn std::error::Error>>(()),
                Err(e) = run_tcp_forever(addr) => Err(e.into()),
            }?
        }
        #[cfg(unix)]
        Some("--listen-unix") => {
            init_logger();
            let path = args
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(default_unix_socket_path);
            tokio::select! {
                _ = shutdown.recv() => Ok::<(), Box<dyn std::error::Error>>(()),
                Err(e) = run_unix_forever(path) => Err(e.into()),
            }?
        }
        #[cfg(unix)]
        Some("--connect") => {
            init_logger();
            let path = args
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(default_unix_socket_path);
            run_connect(path, shutdown).await?;
        }
        Some("--deploy") => run_deploy()?,
        Some("--sync") => run_sync()?,
        _ => usage(),
    }
    Ok(())
}

fn init_logger() {
    let config = load_from_file(server_config_path());
    let log_dir = utils::expand_tilde(&config.log_dir);
    logger::init(log_dir);
}

fn usage() {
    println!("rime_ls v{}", env!("CARGO_PKG_VERSION"));
    println!("Usage: rime_ls [--listen <bind_addr>] [--listen-unix [socket_path]] [--connect [socket_path]] [--deploy] [--sync]");
    println!("  --listen        Listen on TCP (default: 127.0.0.1:9257)");
    #[cfg(unix)]
    {
        println!("  --listen-unix   Listen on Unix socket (default: $XDG_RUNTIME_DIR/rime-ls.sock)");
        println!("  --connect       Connect to rime-ls server via Unix socket (client mode)");
    }
    println!("  --deploy        Deploy Rime (build schema, etc.) using user_data_dir from config");
    println!("  --sync          Sync Rime user data");
}

#[tokio::main]
async fn main() {
    // tell things to shutdown
    let (tx, rx) = broadcast::channel(1);
    // waiting for ctrl-c
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        let _ = tx.send(());
    });
    // run
    if let Err(e) = run(rx).await {
        logger::error(format!("{e}"));
    }
    // finalize rime if necessary
    if Rime::is_initialized() {
        Rime::global().destroy();
    }
}
