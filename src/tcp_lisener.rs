use crate::syscall;
use log::info;
use my_error::{anyhow, d, bail, MyError, MyResultTrait, Result};
use std::future::Future;
use std::os::unix::io::RawFd;
use std::pin::Pin;
use std::task::{Context, Poll};
use super::REACTOR;
use super::epoll::EpollEventType;

pub struct Ipv4Addr(libc::in_addr);
pub struct TcpListener(RawFd);
#[derive(Clone)]
pub struct TcpStream(pub RawFd);
pub struct Incoming<'a>(&'a TcpListener);

impl Ipv4Addr {
    pub fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Ipv4Addr(libc::in_addr {
            s_addr: ((u32::from(a) << 24)
                | (u32::from(b) << 16)
                | (u32::from(c) << 8)
                | u32::from(d))
            .to_be(),
        })
    }
}

pub struct AcceptFuture<'a>(&'a TcpListener);
pub struct ReadFuture<'a>(&'a TcpStream, &'a mut [u8]);
pub struct WriteFuture<'a>(&'a TcpStream, &'a [u8]);

impl TcpListener {
    pub fn bind(addr: Ipv4Addr, port: u16) -> Result<TcpListener> {
        // create a socket
        let sock = syscall!(socket(
            libc::PF_INET,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC,
            0
        ))
        .c(d!())?;

        // set opt
        let opt: i32 = 1;
        syscall!(setsockopt(
            sock,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &opt as *const i32 as *const libc::c_void,
            std::mem::size_of_val(&opt) as u32
        ))
        .c(d!())?;

        // set sin
        let sin: libc::sockaddr_in = libc::sockaddr_in {
            sin_family: libc::AF_INET as libc::sa_family_t,
            sin_port: port.to_be(),
            sin_addr: addr.0,
            sin_zero: [0u8; 8],
        };
        let addrp: *const libc::sockaddr = &sin as *const _ as *const _;
        let len = std::mem::size_of_val(&sin) as libc::socklen_t;

        // bind
        syscall!(bind(sock, addrp, len)).c(d!())?;

        // listen, set backlog
        let backlog = 128;
        syscall!(listen(sock, backlog)).c(d!())?;

        info!("(TcpListner) listen: {}", sock);
        let listner = TcpListener(sock);
        listner.setnonblocking()?;
        Ok(listner)
    }

    pub fn accept(&self) -> Result<TcpStream> {
        let mut sin_client: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        let addrp: *mut libc::sockaddr = &mut sin_client as *mut _ as *mut _;
        let mut len: libc::socklen_t = unsafe { std::mem::zeroed() };
        let lenp: *mut _ = &mut len as *mut _;
        let sock_client = syscall!(accept(self.0, addrp, lenp)).c(d!())?;
        info!("(TcpStream)  accept: {}", sock_client);
        Ok(TcpStream(sock_client))
    }

    pub fn incoming<'a>(&'a self) -> Incoming<'a> {
        Incoming(self)
    }

    pub fn setnonblocking(&self) -> Result<()> {
        let flag = syscall!(fcntl(self.0, libc::F_GETFL, 0)).c(d!())?;
        syscall!(fcntl(
            self.0,
            libc::F_SETFL,
            flag | libc::O_NONBLOCK
        ))
        .c(d!())?;
        Ok(())
    }
}

impl<'a> Incoming<'a> {
    // TODO: copy to  TcpListener? and remove this Incoming?
    pub fn next(&self) -> AcceptFuture<'a> {
        AcceptFuture(self.0)
    }
}

impl<'a> Future for AcceptFuture<'a> {
    type Output = Option<Result<TcpStream>>;

    // A newly created task is called `poll` method by executor immediately.
    // if luckly, syscall success, else syscall will return `would block` error code.
    // we then will register an intresting event to epoll.
    fn poll<'b>(self: Pin<&mut Self>, cx: &mut Context<'b>) -> Poll<Self::Output> {
        match self.0.accept() {
            Ok(stream) => {
                let stream: TcpStream = stream;
                stream.setnonblocking()?;
                Poll::Ready(Some(Ok(stream)))
            }
            Err(e) => {
                let e: MyError = e;
                if let Some(e) = e.get_root_error().downcast_ref::<std::io::Error>() {
                    let e: &std::io::Error = e;
                    if e.kind() == std::io::ErrorKind::WouldBlock {
                        REACTOR.add_event((self.0).0, EpollEventType::In, cx.waker().clone())?;
                        return Poll::Pending;
                    }
                }
                Poll::Ready(Some(Err(e)))
            }
        }
    }
}

impl TcpStream {
    pub fn setnonblocking(&self) -> Result<()> {
        let flag = syscall!(fcntl(self.0, libc::F_GETFL, 0)).c(d!())?;
        syscall!(fcntl(
            self.0,
            libc::F_SETFL,
            flag | libc::O_NONBLOCK
        ))
        .c(d!())?;
        Ok(())
    }

    pub fn read<'a>(&'a self, buf: &'a mut[u8]) -> ReadFuture<'a> {
        ReadFuture(self, buf)
    }
    pub fn write<'a>(&'a self, buf: &'a [u8]) -> WriteFuture<'a> {
        WriteFuture(self, buf)
    }
}

impl<'a> Future for ReadFuture<'a> {
    type Output = Result<usize>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let res = syscall!(read(
            (self.0).0,
            self.1.as_mut_ptr() as *mut libc::c_void,
            self.1.len()
        ));
        match res {
            Ok(n) => Poll::Ready(Ok(n as usize)),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                REACTOR.add_event((self.0).0, EpollEventType::In, cx.waker().clone())?;
                Poll::Pending
            }
            Err(e) => Poll::Ready(bail!(e))
        }
    
    }
}

impl<'a> Future for WriteFuture<'a> {
    type Output = Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let res = syscall!(write(
            (self.0).0,
            self.1.as_ptr() as *mut libc::c_void,
            self.1.len()
        ));
        match res {
            Ok(n) => Poll::Ready(Ok(n as usize)),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                REACTOR.add_event((self.0).0, EpollEventType::Out, cx.waker().clone())?;
                Poll::Pending
            }
            Err(e) => Poll::Ready(bail!(e)),
        }
    }
}

impl Drop for TcpListener {
    fn drop(&mut self) {
        info!("(TcpListner) close : {}", self.0);
        let _ = syscall!(close(self.0));
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        info!("(TcpStream)  close : {}", self.0);
        let _ = syscall!(close(self.0));
    }
}

