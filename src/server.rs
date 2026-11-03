use std::fs;
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{error, info};

pub struct DashboardServer;

impl DashboardServer {
    pub async fn run(port: u16, html_path: &Path) -> anyhow::Result<()> {
        let addr = format!("0.0.0.0:{}", port);
        let listener = TcpListener::bind(&addr).await?;
        info!("============================================================");
        info!("🪟 Windows 95 Dashboard live at http://127.0.0.1:{}", port);
        info!("Press Ctrl+C to terminate the dashboard server.");
        info!("============================================================");

        let html_content = match fs::read_to_string(html_path) {
            Ok(c) => c,
            Err(e) => {
                error!("Could not read dashboard html file: {}", e);
                format!("<h1>Error reading dashboard file: {}</h1>", e)
            }
        };

        loop {
            match listener.accept().await {
                Ok((mut socket, _)) => {
                    let content = html_content.clone();
                    tokio::spawn(async move {
                        let mut buf = [0u8; 1024];
                        let _ = socket.read(&mut buf).await;

                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            content.len(),
                            content
                        );

                        let _ = socket.write_all(response.as_bytes()).await;
                        let _ = socket.flush().await;
                    });
                }
                Err(e) => {
                    error!("TCP accept error: {}", e);
                }
            }
        }
    }
}

