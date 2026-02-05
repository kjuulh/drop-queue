use std::sync::{Arc, atomic::AtomicBool};

use tokio::sync::{
    Mutex,
    mpsc::{self, UnboundedReceiver, UnboundedSender},
};

#[cfg(feature = "notmad")]
use tokio_util::sync::CancellationToken;

type ThreadSafeQueueItem = Arc<Mutex<dyn QueueItem + Send + Sync + 'static>>;

#[derive(Clone)]
pub struct DropQueue {
    draining: Arc<AtomicBool>,
    input: UnboundedSender<ThreadSafeQueueItem>,
    receiver: Arc<Mutex<UnboundedReceiver<ThreadSafeQueueItem>>>,
}

impl Default for DropQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl DropQueue {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        Self {
            draining: Arc::new(AtomicBool::new(false)),
            input: tx,
            receiver: Arc::new(Mutex::new(rx)),
        }
    }

    pub fn assign<F, Fut>(&self, f: F) -> anyhow::Result<()>
    where
        F: FnOnce() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        if self.draining.load(std::sync::atomic::Ordering::Relaxed) {
            panic!("trying to put an item on a draining queue. This is not allowed");
        }

        self.input
            .send(Arc::new(Mutex::new(ClosureComponent {
                inner: Box::new(Some(f)),
            })))
            .expect("unbounded channel should never be full");

        Ok(())
    }

    pub async fn process_next(&self) -> anyhow::Result<()> {
        let item = {
            let mut queue = self.receiver.lock().await;
            queue.recv().await
        };

        if let Some(item) = item {
            let mut item = item.try_lock().expect("should always be unlockable");
            item.execute().await?;
        }

        Ok(())
    }

    pub async fn process(&self) -> anyhow::Result<()> {
        loop {
            if self.draining.load(std::sync::atomic::Ordering::Relaxed) {
                return Ok(());
            }

            self.process_next().await?;
        }
    }

    pub async fn try_process_next(&self) -> anyhow::Result<Option<()>> {
        let item = {
            let mut queue = self.receiver.lock().await;
            match queue.try_recv() {
                Ok(o) => o,
                Err(_) => return Ok(None),
            }
        };

        let mut item = item
            .try_lock()
            .expect("we should always be able to unlock item");
        item.execute().await?;

        Ok(Some(()))
    }

    pub async fn drain(&self) -> anyhow::Result<()> {
        self.draining
            .store(true, std::sync::atomic::Ordering::Release);

        while self.try_process_next().await?.is_some() {}

        Ok(())
    }

    #[cfg(feature = "notmad")]
    async fn process_all(&self, cancellation_token: CancellationToken) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    break;
                },
                res = self.process_next() => {
                    res?;
                }
            }
        }

        self.drain().await?;

        Ok(())
    }
}

struct ClosureComponent<F, Fut>
where
    F: FnOnce() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), anyhow::Error>> + Send + 'static,
{
    inner: Box<Option<F>>,
}

#[async_trait::async_trait]
trait QueueItem {
    async fn execute(&mut self) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
impl<F, Fut> QueueItem for ClosureComponent<F, Fut>
where
    F: FnOnce() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), anyhow::Error>> + Send + 'static,
{
    async fn execute(&mut self) -> Result<(), anyhow::Error> {
        let item = self.inner.take().expect("to only be called once");

        item().await?;

        Ok(())
    }
}

#[cfg(feature = "notmad")]
mod notmad {
    use notmad::{ComponentInfo, MadError};
    use tokio_util::sync::CancellationToken;

    use crate::DropQueue;

    impl notmad::Component for DropQueue {
        fn info(&self) -> ComponentInfo {
            "drop-queue/drop-queue".into()
        }

        async fn run(&self, cancellation_token: CancellationToken) -> Result<(), MadError> {
            self.process_all(cancellation_token)
                .await
                .map_err(notmad::MadError::Inner)?;

            Ok(())
        }
    }
}
#[cfg(feature = "notmad")]
#[allow(unused_imports)]
pub use notmad::*;

#[cfg(test)]
mod test {
    use tokio::sync::oneshot;

    use crate::DropQueue;

    #[tokio::test]
    async fn can_drop_item() -> anyhow::Result<()> {
        let drop_queue = DropQueue::new();

        let (called_tx, called_rx) = oneshot::channel();

        drop_queue.assign(|| async move {
            tracing::info!("was called");

            called_tx.send(()).unwrap();

            Ok(())
        })?;

        drop_queue.process_next().await?;

        called_rx.await?;

        Ok(())
    }

    #[tokio::test]
    async fn can_drop_multiple_items() -> anyhow::Result<()> {
        let drop_queue = DropQueue::new();

        let (called_tx, called_rx) = oneshot::channel();
        let _drop_queue = drop_queue.clone();
        tokio::spawn(async move {
            _drop_queue
                .assign(|| async move {
                    tracing::info!("was called");

                    called_tx.send(()).unwrap();

                    Ok(())
                })
                .unwrap();
        });

        let (called_tx2, called_rx2) = oneshot::channel();
        let _drop_queue = drop_queue.clone();
        tokio::spawn(async move {
            _drop_queue
                .assign(|| async move {
                    tracing::info!("was called");

                    called_tx2.send(()).unwrap();

                    Ok(())
                })
                .unwrap();
        });

        drop_queue.process_next().await?;
        drop_queue.process_next().await?;

        called_rx.await?;
        called_rx2.await?;

        Ok(())
    }
}
