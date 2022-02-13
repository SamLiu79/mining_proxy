use anyhow::Result;
use log::info;
use tokio::{
    io::{split, BufReader},
    net::{TcpListener, TcpStream},
    sync::mpsc::UnboundedSender,
};

use crate::{
    state::{State, Worker},
    util::config::Settings,
};

use super::*;
pub async fn accept_en_tcp(
    worker_sender: UnboundedSender<Worker>, config: Settings, state: State,
) -> Result<()> {
    if config.encrypt_port == 0 {
        return Ok(());
    }

    let address = format!("0.0.0.0:{}", config.encrypt_port);
    let listener = match TcpListener::bind(address.clone()).await {
        Ok(listener) => listener,
        Err(_) => {
            log::info!("本地端口被占用 {}", address);
            std::process::exit(1);
        }
    };

    log::info!("本地TCP加密协议端口{}启动成功!!!", &address);
    loop {
        let (stream, addr) = listener.accept().await?;

        let config = config.clone();
        let workers = worker_sender.clone();
        let state = state.clone();
        state
            .online
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // 在这里初始化矿工信息。传入spawn. 然后退出的时候再进行矿工下线通知。

        tokio::spawn(async move {
            // 矿工状态管理
            let mut worker: Worker = Worker::default();
            match transfer(
                &mut worker,
                workers.clone(),
                stream,
                &config,
                state.clone(),
            )
            .await
            {
                Ok(_) => {
                    state
                        .online
                        .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    if worker.is_online() {
                        worker.offline();
                        workers.send(worker);
                    } else {
                        info!("IP: {} 断开", addr);
                    }
                }
                Err(e) => {
                    if worker.is_online() {
                        worker.offline();
                        workers.send(worker);
                        info!("IP: {} 断开原因 {}", addr, e);
                    } else {
                        debug!("IP: {} 恶意链接断开: {}", addr, e);
                    }

                    state
                        .online
                        .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
        });
    }
}

async fn transfer(
    worker: &mut Worker, worker_queue: UnboundedSender<Worker>,
    tcp_stream: TcpStream, config: &Settings, state: State,
) -> Result<()> {
    let (worker_r, worker_w) = split(tcp_stream);
    let worker_r = BufReader::new(worker_r);
    let (stream_type, pools) =
        match crate::client::get_pool_ip_and_type(&config) {
            Ok(pool) => pool,
            Err(e) => {
                bail!("未匹配到矿池 或 均不可链接。请修改后重试");
            }
        };

    if config.share == 0 {
        handle_tcp_pool(
            worker,
            worker_queue,
            worker_r,
            worker_w,
            &pools,
            &config,
            state,
            true,
        )
        .await
    } else if config.share == 1 {
        if config.share_alg == 99 {
            handle_tcp_random(
                worker,
                worker_queue,
                worker_r,
                worker_w,
                &pools,
                &config,
                state,
                true,
            )
            .await
        } else {
            handle_tcp_pool_timer(
                worker,
                worker_queue,
                worker_r,
                worker_w,
                &pools,
                &config,
                state,
                true,
            )
            .await
        }
    } else {
        handle_tcp_pool_all(
            worker,
            worker_queue,
            worker_r,
            worker_w,
            &config,
            state,
            true,
        )
        .await
    }
}
