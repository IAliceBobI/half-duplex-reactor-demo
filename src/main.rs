use crate::epoll::Epoll;
use crate::executor::new_executor_and_spawner;
use crate::tcp_lisener::Ipv4Addr;
use crate::tcp_lisener::TcpListener;
use lazy_static::lazy_static;
use log::info;
use my_error::Result;
use std::collections::HashMap;
use std::io::Write;
use std::sync::Mutex;
use tcp_lisener::TcpStream;

#[macro_use]
mod util;
mod epoll;
mod executor;
mod reactor;
mod tcp_lisener;

use reactor::{reactor_main_loop, Reactor};

lazy_static! {
    static ref REACTOR: Reactor = {
        std::thread::spawn(move || reactor_main_loop());

        Reactor {
            epoll: Epoll::new().expect("Failed to create epoll"),
            wakers: Mutex::new(HashMap::new()),
        }
    };
}

async fn handle_client(stream: TcpStream) -> Result<()> {
    let mut buf = [0u8; 1024];
    info!("(handle client) {}", stream.0);
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        stream.write(b"FromServer: ").await?;
        stream.write(&buf[..n]).await?;
    }
    Ok(())
}

async fn keepalive_client(stream: TcpStream) -> Result<()> {
    for _ in 0..20 {
        stream.write(b"pingFromServer").await?;
    }
    Ok(())
}

fn init_log() {
    // format = [file:line] msg
    std::env::set_var("RUST_LOG", "info");
    env_logger::Builder::from_default_env()
        .format(|buf, record| {
            writeln!(
                buf,
                "[{}:{:>3}] {}",
                record.file().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                record.args(),
            )
        })
        .init();
}

fn main() {
    init_log();
    let (executor, spawner) = new_executor_and_spawner();
    let spawner_clone = spawner.clone();
    let mainloop = async move {
        let addr = Ipv4Addr::new(0, 0, 0, 0);
        let port = 8080;
        let listner = TcpListener::bind(addr, port)?;

        let incoming = listner.incoming();

        while let Some(stream) = incoming.next().await {
            let stream = stream?;
            spawner.spawn(handle_client(stream));
            // spawner.spawn(keepalive_client(stream));
        }

        Ok(())
    };

    spawner_clone.spawn(mainloop);
    drop(spawner_clone);
    executor.run();
}
