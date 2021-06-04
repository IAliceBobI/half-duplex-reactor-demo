use super::epoll::{Epoll, EpollEventType};
use super::REACTOR;
use log::info;
use my_error::Result;
use std::collections::HashMap;
use std::os::unix::io::RawFd;
use std::sync::Mutex;
use std::task::Waker;

pub struct Reactor {
    pub epoll: Epoll,
    pub wakers: Mutex<HashMap<RawFd, Waker>>,
    //TODO: pub wakers: Mutex<HashMap<(RawFd, EpollEventType), Waker>>,
    // pub events: Mutex<HashMap<RawFd, u32>>,
}

impl Reactor {
    pub fn add_event(&self, fd: RawFd, op: EpollEventType, waker: Waker) -> Result<()> {
        info!("(Reactor) add event: {}", fd);
        self.epoll.add_event(fd, op)?;
        self.wakers.lock().unwrap().insert(fd, waker);
        //self.wakers.lock().unwrap().insert((fd, op), waker);
        Ok(())
    }
}

pub fn reactor_main_loop() -> Result<()> {
    info!("Start reactor main loop");
    // `events` only modified by syscall
    // not by our code.
    let mut events: Vec<libc::epoll_event> = vec![unsafe { std::mem::zeroed() }; 32];
    let reactor = &REACTOR;

    loop {
        // TODO:
        // wait in reactor mode is different from other?
        let nfd = reactor.epoll.wait(&mut events)?;
        info!("(Reactor) wake up. nfd={}", nfd);
        for i in 0..nfd {
            // typedef union epoll_data {
            //     void    *ptr;
            //     int      fd;
            //     uint32_t u32;
            //     uint64_t u64;
            // } epoll_data_t;
            // struct epoll_event {
            //     uint32_t     events;    /* Epoll events */
            //     epoll_data_t data;      /* User data variable */
            // };
            let fd = events[i].u64 as RawFd;
            // let eventflags = events[i].events;
            // if eventflags as i32 & libc::EPOLLIN == libc::EPOLLIN {
            // }
            // if eventflags as i32 & libc::EPOLLOUT == libc::EPOLLOUT {
            // } 
            let waker = reactor
                .wakers
                .lock()
                .unwrap()
                .remove(&fd)
                .unwrap_or_else(|| panic!("not found fd {}", fd));
            info!("(Reactor) delete event: {}", fd);
            reactor.epoll.del_event(fd)?;
            waker.wake();
        }
    }
}
