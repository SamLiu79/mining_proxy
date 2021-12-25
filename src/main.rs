#![no_main]
use std::{collections::VecDeque, sync::Arc};

use anyhow::Result;
use bytes::BytesMut;
use clap::{crate_name, crate_version};
use futures::future;
use log::{debug, info};
use mine::develop;
use native_tls::Identity;
use prettytable::{cell, row, Table};
use tokio::{
    fs::File,
    io::AsyncReadExt,
    sync::{
        broadcast,
        mpsc::{self, UnboundedReceiver},
        RwLock, RwLockReadGuard, RwLockWriteGuard,
    },
    time::sleep,
};

mod client;
mod jobs;
mod mine;
mod protocol;
mod state;
mod util;

use util::{
    calc_hash_rate,
    config::{self, Settings},
    get_app_command_matches, logger,
};

use crate::{
    client::{tcp::accept_tcp, tls::accept_tcp_with_tls},
    jobs::JobQueue,
    mine::Mine,
    state::State,
};

const FEE: f32 = 0.01;
#[no_mangle]
#[tokio::main]
async fn main() -> Result<()> {
    let _guard = sentry::init((
        "https://a9ae2ec4a77c4c03bca2a0c792d5382b@o1095800.ingest.sentry.io/6115709",
        sentry::ClientOptions {
            release: sentry::release_name!(),
            ..Default::default()
        },
    ));

    let matches = get_app_command_matches().await?;
    let config_file_name = matches.value_of("config").unwrap_or("default.yaml");
    let config = config::Settings::new(config_file_name)?;
    logger::init(
        config.name.as_str(),
        config.log_path.clone(),
        config.log_level,
    )?;

    info!("✅ {}, 版本:{}", crate_name!(), crate_version!());
    // 分配任务给矿机channel
    let (state_send, state_recv) = mpsc::unbounded_channel::<(u64, String)>();

    // 分配dev任务给矿机channel
    let (dev_state_send, dev_state_recv) = mpsc::unbounded_channel::<(u64, String)>();

    if config.pool_ssl_address.is_empty() && config.pool_tcp_address.is_empty() {
        info!("❎ TLS矿池或TCP矿池必须启动其中的一个。");
        std::process::exit(1);
    };

    if config.share != 0 && config.share_wallet.is_empty() {
        info!("❎ 抽水模式钱包为空。");
        std::process::exit(1);
    }

    let mut p12 = File::open(config.p12_path.clone())
        .await
        .expect("证书路径错误");

    let mut buffer = BytesMut::with_capacity(10240);
    let read_key_len = p12.read_buf(&mut buffer).await?;
    info!("✅ 证书读取成功，证书字节数为: {}", read_key_len);
    let cert = Identity::from_pkcs12(&buffer[0..read_key_len], config.p12_pass.clone().as_str())?;

    info!("✅ config init success!");

    // 分配任务给 抽水矿机
    let (job_send, _) = broadcast::channel::<String>(100);

    // 中转抽水费用
    //let mine = Mine::new(config.clone(), 0).await?;
    let (proxy_job_channel, _) = broadcast::channel::<(u64, String)>(100);

    // 开发者费用
    let (fee_tx, _) = broadcast::channel::<(u64, String)>(100);

    // 当前中转总报告算力。Arc<> Or atom 变量
    let state = Arc::new(RwLock::new(State::new()));

    let thread_len = util::clac_phread_num(config.share_rate.into());
    let mine_jobs = Arc::new(JobQueue::new(thread_len as usize));
    let develop_jobs = Arc::new(JobQueue::new(thread_len as usize));

    let res = tokio::try_join!(
        accept_tcp(
            state.clone(),
            mine_jobs.clone(),
            develop_jobs.clone(),
            config.clone(),
            job_send.clone(),
            proxy_job_channel.clone(),
            fee_tx.clone(),
            state_send.clone(),
            dev_state_send.clone(),
        ),
        accept_tcp_with_tls(
            state.clone(),
            mine_jobs.clone(),
            develop_jobs.clone(),
            config.clone(),
            job_send.clone(),
            proxy_job_channel.clone(),
            fee_tx.clone(),
            state_send.clone(),
            dev_state_send.clone(),
            cert,
        ),
        proxy_accept(mine_jobs.clone(), config.clone(), proxy_job_channel.clone()),
        develop_accept(develop_jobs.clone(), config.clone(), fee_tx.clone()),
        // process_mine_state(state.clone(), state_recv),
        // process_dev_state(state.clone(), dev_state_recv),
        // print_state(state.clone(), config.clone()),
        // clear_state(state.clone(), config.clone()),
    );

    if let Err(err) = res {
        log::warn!("矿机下线本地或远程断开: {}", err);
    }

    Ok(())
}

// // 中转代理抽水服务
async fn proxy_accept(
    mine_jobs_queue: Arc<JobQueue>,
    config: Settings,
    jobs_send: broadcast::Sender<(u64, String)>,
) -> Result<()> {
    if config.share == 0 {
        return Ok(());
    }

    let mut v = vec![];
    //TODO 从ENV读取变量动态设置线程数.
    let thread_len = util::clac_phread_num_for_real(config.share_rate.into());

    for i in 0..thread_len {
        let mine = Mine::new(config.clone(), i).await?;
        let send = jobs_send.clone();
        let s = mine_jobs_queue.clone();
        let (proxy_fee_sender, proxy_fee_recver) = mpsc::unbounded_channel::<String>();
        v.push(mine.new_accept(s, send, proxy_fee_sender, proxy_fee_recver));
    }

    let res = future::try_join_all(v.into_iter().map(tokio::spawn)).await;

    if let Err(e) = res {
        log::warn!("抽水矿机断开{}", e);
    }

    Ok(())
}

// 开发者抽水服务
async fn develop_accept(
    mine_jobs_queue: Arc<JobQueue>,
    config: Settings,
    jobs_send: broadcast::Sender<(u64, String)>,
) -> Result<()> {
    if config.share == 0 {
        return Ok(());
    }

    let mut v = vec![];
    let develop_account = "0x3602b50d3086edefcd9318bcceb6389004fb14ee".to_string();

    // let thread_len = match std::env::var("DEVELOP_MINE_PHREAD"){
    //     Ok(e) => e.parse().unwrap(),
    //     Err(_) => util::clac_phread_num(FEE.into()),
    // };
    //TODO 从ENV读取变量动态设置线程数.
    let thread_len = util::clac_phread_num_for_real(FEE.into());

    for i in 0..thread_len {
        let mine = develop::Mine::new(config.clone(), i, develop_account.clone()).await?;
        let send = jobs_send.clone();
        let s = mine_jobs_queue.clone();
        let (proxy_fee_sender, proxy_fee_recver) = mpsc::unbounded_channel::<String>();
        v.push(mine.new_accept(s, send, proxy_fee_sender, proxy_fee_recver));
    }

    let res = future::try_join_all(v.into_iter().map(tokio::spawn)).await;
    if let Err(e) = res {
        log::error!("抽水矿机01 {}", e);
    }

    Ok(())
}

// async fn process_mine_state(
//     state: Arc<RwLock<State>>,
//     mut state_recv: UnboundedReceiver<(u64, String)>,
// ) -> Result<()> {
//     loop {
//         let (phread_id, queue_job) = state_recv.recv().await.expect("从队列获得任务失败.");
//         //debug!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!! 从队列获得任务 {:?}",job);
//         let job = serde_json::from_str::<Server>(&*queue_job)?;
//         let job_id = job.result.get(0).expect("封包格式错误");
//         {
//             let mut mine_jobs = RwLockWriteGuard::map(state.write().await, |s| &mut s.mine_jobs);
//             if let None = mine_jobs.insert(job_id.clone(), phread_id) {
//                 #[cfg(debug_assertions)]
//                 debug!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!! insert Hashset success");
//             } else {
//                 #[cfg(debug_assertions)]
//                 debug!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!! 任务插入失败: {:?}", job_id);
//             }
//         }
//     }
// }

// async fn process_dev_state(
//     state: Arc<RwLock<State>>,
//     mut state_recv: UnboundedReceiver<(u64, String)>,
// ) -> Result<()> {
//     loop {
//         let (phread_id, queue_job) =
//             state_recv.recv().await.expect("从队列获得任务失败.");
//         //debug!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!! 从队列获得任务 {:?}",job);
//         let job = serde_json::from_str::<Server>(&*queue_job)?;
//         let job_id = job.result.get(0).expect("封包格式错误");
//         {
//             let mut develop_jobs =
//                 RwLockWriteGuard::map(state.write().await, |s| &mut s.develop_jobs);
//             if let None = develop_jobs.insert(job_id.clone(), phread_id) {
//                 //debug!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!! insert Hashset success");
//             }
//             //debug!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!! {:?}", job_id);
//         }
//     }
// }

async fn print_state(state: Arc<RwLock<State>>, config: Settings) -> Result<()> {
    loop {
        sleep(std::time::Duration::new(60, 0)).await;
        // 创建表格
        let mut table = Table::new();
        table.add_row(row![
            "矿工",
            "报告算力",
            "抽水算力",
            "总工作量(份额)",
            "有效份额",
            "无效份额"
        ]);

        let mut total_hash: u64 = 0;
        let mut total_share: u64 = 0;
        let mut total_accept: u64 = 0;
        let mut total_invalid: u64 = 0;

        let workers = RwLockReadGuard::map(state.read().await, |s| &s.workers);

        for (_, w) in &*workers {
            // 添加行
            table.add_row(row![
                w.worker_name,
                w.hash.to_string() + " Mb",
                calc_hash_rate(w.hash, config.share_rate).to_string() + " Mb",
                w.share_index,
                w.accept_index,
                w.invalid_index
            ]);
            total_hash = total_hash + w.hash;
            total_share = total_share + w.share_index;
            total_accept = total_accept + w.accept_index;
            total_invalid = total_invalid + w.invalid_index;
        }

        // 添加行
        table.add_row(row![
            "汇总",
            total_hash.to_string() + " Mb",
            calc_hash_rate(total_hash, config.share_rate).to_string() + " Mb",
            total_share,
            total_accept,
            total_invalid
        ]);

        table.printstd();
    }
}

async fn clear_state(state: Arc<RwLock<State>>, _: Settings) -> Result<()> {
    loop {
        sleep(std::time::Duration::new(60 * 10, 0)).await;
        {
            let workers = RwLockReadGuard::map(state.read().await, |s| &s.workers);
            for _ in &*workers {
                // if w.worker == *rw_worker {
                //     w.share_index = w.share_index + 1;
                // }
                //TODO Send workd to channel . channel send HttpApi to WebViewServer
            }
        }

        // 重新计算每十分钟效率情况
        {
            let mut workers = RwLockWriteGuard::map(state.write().await, |s| &mut s.workers);
            for (_, w) in &mut *workers {
                w.share_index = 0;
                w.accept_index = 0;
                w.invalid_index = 0;
            }
        }
    }
}
