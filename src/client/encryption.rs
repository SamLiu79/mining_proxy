#![allow(dead_code)]
#![allow(unused)]

use std::{net::SocketAddr, time::Duration};

use anyhow::{bail, Result};
use log::{debug, info};

use openssl::symm::{decrypt, encrypt, Cipher};

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    select,
};

use crate::client::{self_write_socket_byte, write_to_socket_byte};

pub async fn accept_encrypt_tcp(
    port: i32, server: SocketAddr, key: Vec<u8>, iv: Vec<u8>,
) -> Result<()> {
    let address = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(address.clone()).await?;
    //info!("😄 Accepting EncryptData Tcp On: {}", &address);

    println!("本地加密协议端口{}启动成功!!!", &address);

    loop {
        let (stream, addr) = listener.accept().await?;
        info!("😄 Accepting EncryptData Tcp connection from {}", addr);
        let iv = iv.clone();
        let key = key.clone();

        tokio::spawn(async move { transfer(stream, server, key, iv).await });
    }
}

async fn transfer(
    stream: TcpStream, addr: SocketAddr, key: Vec<u8>, iv: Vec<u8>,
) -> Result<()> {
    let (worker_r, mut worker_w) = tokio::io::split(stream);
    let worker_r = tokio::io::BufReader::new(worker_r);
    let mut worker_r = worker_r.lines();

    let std_stream = match std::net::TcpStream::connect_timeout(
        &addr,
        Duration::new(5, 0),
    ) {
        Ok(stream) => stream,
        Err(_) => {
            bail!("{} 远程地址不通！", addr);
        }
    };

    std_stream.set_nonblocking(true).unwrap();
    let pool_stream = TcpStream::from_std(std_stream)?;
    let (pool_r, mut pool_w) = tokio::io::split(pool_stream);
    let pool_r = tokio::io::BufReader::new(pool_r);
    let mut pool_r = pool_r.split(crate::SPLIT);
    let mut client_timeout_sec = 1;

    let key = key.clone();
    let iv = iv.clone();

    loop {
        select! {
            res = worker_r.next_line() => {
                let buffer = match res{
                    Ok(res) => match res{
                        Some(buf) => buf,
                        None => {
                            pool_w.shutdown().await;
                            bail!("矿机下线了")
                        }
                    },
                    Err(e) => {pool_w.shutdown().await; bail!("读取超时了 矿机下线了: {}",e)},
                };

                #[cfg(debug_assertions)]
                debug!("------> :  矿机 -> 矿池  {:?}", buffer);
                let buffer: Vec<_> = buffer.split("\n").collect();
                for buf in buffer {
                    if buf.is_empty() {
                        continue;
                    }
                    let cipher = Cipher::aes_256_cbc();
                    //let data = b"Some Crypto String";
                    let ciphertext = encrypt(
                        cipher,
                        &key,
                        Some(&iv),
                        buf.as_bytes()).unwrap();

                    let base64 = base64::encode(&ciphertext[..]);

                    match self_write_socket_byte(&mut pool_w,base64.as_bytes().to_vec(),&"加密".to_string()).await{
                        Ok(_) => {},
                        Err(e) => {info!("{}",e);bail!("矿机下线了 {}",e)}
                    }
                }
            },
            res = pool_r.next_segment() => {
                let _start = std::time::Instant::now();
                let buffer = match res{
                    Ok(res) => {
                        match res {
                            Some(buf) => buf,
                            None => {
                                worker_w.shutdown().await;

                                bail!("矿机下线了")
                            }
                        }
                    },
                    Err(e) => {bail!("矿机下线了: {}",e)},
                };


                #[cfg(debug_assertions)]
                debug!("<------ :  矿池 -> 矿机  {:?}", buffer);

                let buffer = buffer[0..buffer.len()].split(|c| *c == crate::SPLIT);
                for buf in buffer {
                    if buf.is_empty() {
                        continue;
                    }

                    let buf = match base64::decode(&buf[..]) {
                        Ok(buf) => buf,
                        Err(e) => {
                            log::error!("{}",e);
                            pool_w.shutdown().await;
                            return Ok(());
                        },
                    };

                    let cipher = Cipher::aes_256_cbc();
                    // 解密
                    let buffer = match decrypt(
                        cipher,
                        &key,
                        Some(&iv),
                        &buf[..]) {
                            Ok(s) => s,
                            Err(e) => {
                                info!("解密失败 {}",e);
                                pool_w.shutdown().await;
                                return Ok(());
                            },
                        };

                    match write_to_socket_byte(&mut worker_w,buffer,&"解密".to_string()).await{
                        Ok(_) => {},
                        Err(e) => {info!("{}",e);bail!("矿机下线了 {}",e)}
                    }
                }
            }
        }
    }
}
