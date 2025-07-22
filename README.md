# 🧊 nodrop

**`nodrop`** is a simple, composable async drop queue for Rust — built to run queued tasks in FIFO order and ensure graceful shutdown via draining. It's useful in lifecycle-managed environments (e.g., `notmad`) or as a lightweight alternative until async drop is stabilized in Rust.

> 💡 Tasks are executed one at a time. If the queue is marked for draining, no further items can be added.

---

## ✨ Features

* Assign one-off async tasks (closures) to a queue
* FIFO task processing
* Draining mechanism to flush the queue before shutdown
* Optional integration with [`notmad`](https://crates.io/crates/notmad) for graceful component lifecycle control
* `Send + Sync + 'static` guaranteed

---

## 🚀 Usage

### Add to `Cargo.toml`

```toml
[dependencies]
nodrop = "*"
```

### Enable `notmad` integration (optional)

```toml
[dependencies]
nodrop = { version = "*", features = ["notmad"] }
```

---

## 🛠 Example

```rust
use nodrop::DropQueue;
use tokio::sync::oneshot;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let queue = DropQueue::new();

    let (tx, rx) = oneshot::channel();

    queue.assign(|| async move {
        println!("Running closure task");
        tx.send(()).unwrap();
        Ok(())
    })?;

    queue.process_next().await?;

    rx.await?;

    Ok(())
}
```

---

## 🔁 Lifecycle with `notmad`

If using the [`notmad`](https://crates.io/crates/notmad) lifecycle framework:

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let queue = nodrop::DropQueue::new();
    let app = notmad::Mad::new().add(queue);
    app.run().await?;
    Ok(())
}
```

This will process tasks until the cancellation token is triggered, e.g., via SIGINT.

---

## 🔒 Drain Mode

You can signal the queue to stop accepting new items and finish processing the current ones:

```rust
queue.drain().await?;
```

After this call, any further `assign()` will panic.

---

## 🧪 Tests

Run the test suite using:

```bash
cargo test
```

---

## 📜 License

MIT 
