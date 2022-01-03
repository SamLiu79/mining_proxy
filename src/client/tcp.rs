use std::sync::Arc;

use anyhow::Result;
use log::info;

use tokio::io::{split, BufReader};
use tokio::net::{TcpListener, TcpStream};

use tokio::sync::broadcast;

use tokio::sync::mpsc::UnboundedSender;

use crate::jobs::JobQueue;

use crate::state::Worker;
use crate::util::config::Settings;

use super::*;
pub async fn accept_tcp(
    worker_queue: tokio::sync::mpsc::Sender<Worker>,
    mine_jobs_queue: Arc<JobQueue>,
    develop_jobs_queue: Arc<JobQueue>,
    config: Settings,
    _job_send: broadcast::Sender<String>,
    proxy_fee_sender: broadcast::Sender<(u64, String)>,
    develop_fee_sender: broadcast::Sender<(u64, String)>,
    _state_send: UnboundedSender<(u64, String)>,
    _dev_state_send: UnboundedSender<(u64, String)>,
) -> Result<()> {
    let address = format!("0.0.0.0:{}", config.tcp_port);
    let listener = TcpListener::bind(address.clone()).await?;
    info!("😄 Accepting Tcp On: {}", &address);

    loop {
        let (stream, addr) = listener.accept().await?;
        info!("😄 Accepting Tcp connection from {}", addr);

        let config = config.clone();
        let workers = worker_queue.clone();

        let mine_jobs_queue = mine_jobs_queue.clone();
        let develop_jobs_queue = develop_jobs_queue.clone();
        let proxy_fee_sender = proxy_fee_sender.clone();
        let develop_fee_sender = develop_fee_sender.clone();

        tokio::spawn(async move {
            transfer(
                workers,
                stream,
                &config,
                mine_jobs_queue,
                develop_jobs_queue,
                proxy_fee_sender,
                develop_fee_sender,
            )
            .await
        });
    }
}

async fn transfer(
    worker_queue: tokio::sync::mpsc::Sender<Worker>,
    tcp_stream: TcpStream,
    config: &Settings,
    mine_jobs_queue: Arc<JobQueue>,
    develop_jobs_queue: Arc<JobQueue>,
    proxy_fee_sender: broadcast::Sender<(u64, String)>,
    develop_fee_sender: broadcast::Sender<(u64, String)>,
) -> Result<()> {
    let (worker_r, worker_w) = split(tcp_stream);
    let worker_r = BufReader::new(worker_r);
    let (stream_type, pools) = match crate::client::get_pool_ip_and_type(&config) {
        Some(pool) => pool,
        None => {
            info!("未匹配到矿池 或 均不可链接。请修改后重试");
            return Ok(());
        }
    };

    if stream_type == crate::client::TCP {
        handle_tcp_pool(
            worker_queue,
            worker_r,
            worker_w,
            &pools,
            &config,
            mine_jobs_queue,
            develop_jobs_queue,
            proxy_fee_sender,
            develop_fee_sender,
            false,
        )
        .await
    } else if stream_type == crate::client::SSL {
        handle_tls_pool(
            worker_queue,
            worker_r,
            worker_w,
            &pools,
            &config,
            mine_jobs_queue,
            develop_jobs_queue,
            proxy_fee_sender,
            develop_fee_sender,
            false,
        )
        .await
    } else {
        log::error!("致命错误：未找到支持的矿池BUG 请上报");
        return Ok(());
    }
}
