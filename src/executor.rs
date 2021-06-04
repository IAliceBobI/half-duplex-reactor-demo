use futures::future::{BoxFuture, FutureExt};
use futures::task::{waker_ref, ArcWake, WakerRef};
use my_error::Result;
use std::future::Future;
use std::sync::mpsc::sync_channel;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::sync::{Mutex, MutexGuard};
use std::task::{Context, Poll};

pub struct Task {
    future: Mutex<Option<BoxFuture<'static, Result<()>>>>,
    task_sender: SyncSender<Arc<Task>>,
}

impl ArcWake for Task {
    // when a task is spawned, the task is sent into the channel immediately for an executor to run.
    // often, while task is run by an executor, a pending status is returned.
    // it is the current `poll` method's responsibility to add an intresting event to epoll, and
    // registry the task to a map in reactor.
    // when epoll's `wait` method return the event we just set, we find the task regiestried previously
    // and call `wake()` internally call `wake_by_ref()` to send the task back to executor's channel.
    fn wake_by_ref(arc_self: &Arc<Self>) {
        let cloned = arc_self.clone();
        arc_self.task_sender.send(cloned).expect("failed to send");
    }
}

pub struct Executor {
    ready_queue: Receiver<Arc<Task>>,
}

impl Executor {
    pub fn run(&self) {
        while let Ok(task) = self.ready_queue.recv() {
            let mut future_slot: MutexGuard<'_, Option<BoxFuture<'_, _>>> =
                task.future.lock().unwrap();
            if let Some(mut future) = future_slot.take() {
                let waker: WakerRef = waker_ref(&task);
                let mut context = Context::from_waker(&*waker);
                // if let Poll::Pending = future.as_mut().poll(&mut context) {
                //     *future_slot = Some(future);
                // }
                match future.as_mut().poll(&mut context) {
                    Poll::Pending => {
                        *future_slot = Some(future);
                    }
                    Poll::Ready(ret) => {
                        if let Err(e) = ret {
                            log::info!("{}", e);
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct Spawner {
    task_sender: SyncSender<Arc<Task>>,
}

impl Spawner {
    pub fn spawn(&self, fut: impl Future<Output = Result<()>> + 'static + Send) {
        let fut = fut.boxed();
        let task = Arc::new(Task {
            future: Mutex::new(Some(fut)),
            task_sender: self.task_sender.clone(),
        });
        self.task_sender.send(task).expect("failed to send");
    }
}

pub fn new_executor_and_spawner() -> (Executor, Spawner) {
    let (task_sender, ready_queue) = sync_channel(10000);
    (Executor { ready_queue }, Spawner { task_sender })
}
