use std::sync::{Arc, Mutex};

use tokio::io::{AsyncWrite, AsyncWriteExt, BufWriter};
use tokio::sync::{mpsc, oneshot};

use super::{build_xml, csv_escape, flatten_item};
use crate::errors::{SilkwormError, SilkwormResult};
use crate::logging::Logger;
use crate::types::Item;

const QUEUE_CAPACITY: usize = 16;

pub(super) enum RecordFormat {
    JsonLines,
    Csv {
        fieldnames: Option<Vec<String>>,
        header_written: bool,
    },
    Xml {
        item_element: String,
    },
}

impl RecordFormat {
    fn encode(&mut self, item: &Item, buffer: &mut Vec<u8>) -> SilkwormResult<()> {
        buffer.clear();
        match self {
            Self::JsonLines => {
                serde_json::to_writer(&mut *buffer, item)
                    .map_err(|err| SilkwormError::Pipeline(format!("JSON encode failed: {err}")))?;
                buffer.push(b'\n');
            }
            Self::Csv {
                fieldnames,
                header_written,
            } => {
                let flattened = flatten_item(item);
                let names = fieldnames.get_or_insert_with(|| flattened.keys().cloned().collect());
                if !*header_written {
                    append_csv_row(buffer, names.iter().map(String::as_str));
                }
                append_csv_row(
                    buffer,
                    names
                        .iter()
                        .map(|name| flattened.get(name).map_or("", String::as_str)),
                );
            }
            Self::Xml { item_element } => {
                buffer.extend_from_slice(build_xml(item_element, item, 1).as_bytes());
            }
        }
        Ok(())
    }

    fn commit_record(&mut self) {
        if let Self::Csv { header_written, .. } = self {
            *header_written = true;
        }
    }
}

fn append_csv_row<'a>(buffer: &mut Vec<u8>, fields: impl Iterator<Item = &'a str>) {
    for (index, field) in fields.enumerate() {
        if index > 0 {
            buffer.push(b',');
        }
        buffer.extend_from_slice(csv_escape(field).as_bytes());
    }
    buffer.push(b'\n');
}

enum Command {
    Write(Item, oneshot::Sender<SilkwormResult<Item>>),
    Close(oneshot::Sender<SilkwormResult<()>>),
}

/// The actor owns the file for its entire lifetime. Callers only own commands,
/// so dropping a caller cannot drop the writer halfway through a record.
#[derive(Default)]
pub(super) struct FileWriter {
    state: Arc<Mutex<WriterState>>,
}

impl Drop for FileWriter {
    fn drop(&mut self) {
        // The actor also holds the lifecycle state. Remove its stored sender so
        // dropping the pipeline closes the channel instead of retaining a cycle.
        if let Ok(mut state) = self.state.lock() {
            *state = WriterState::Closing;
        }
    }
}

#[derive(Default)]
enum WriterState {
    #[default]
    Closed,
    Opening,
    Open(mpsc::Sender<Command>),
    Closing,
}

pub(super) struct OpeningWriter {
    lifecycle: WriterLifecycle,
}

struct WriterLifecycle {
    state: Arc<Mutex<WriterState>>,
    finished: bool,
}

impl WriterLifecycle {
    fn finish(&mut self) {
        if !self.finished {
            if let Ok(mut state) = self.state.lock() {
                *state = WriterState::Closed;
            }
            self.finished = true;
        }
    }
}

impl Drop for WriterLifecycle {
    fn drop(&mut self) {
        self.finish();
    }
}

impl OpeningWriter {
    pub(super) fn start(
        self,
        file: tokio::fs::File,
        format: RecordFormat,
        footer: Vec<u8>,
        logger: Logger,
    ) -> SilkwormResult<()> {
        self.start_writer(BufWriter::new(file), format, footer, logger)
    }

    fn start_writer<W: AsyncWrite + Unpin + Send + 'static>(
        self,
        writer: W,
        format: RecordFormat,
        footer: Vec<u8>,
        logger: Logger,
    ) -> SilkwormResult<()> {
        let mut state = self
            .lifecycle
            .state
            .lock()
            .map_err(|_| writer_lock_error())?;
        let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
        *state = WriterState::Open(sender);
        drop(state);
        tokio::spawn(run_writer(
            writer,
            format,
            footer,
            receiver,
            logger,
            self.lifecycle,
        ));
        Ok(())
    }
}

impl FileWriter {
    pub(super) fn begin_open(&self) -> SilkwormResult<OpeningWriter> {
        let mut state = self.state.lock().map_err(|_| writer_lock_error())?;
        if !matches!(*state, WriterState::Closed) {
            return Err(SilkwormError::Pipeline(
                "Pipeline already opened or changing state".to_string(),
            ));
        }
        *state = WriterState::Opening;
        Ok(OpeningWriter {
            lifecycle: WriterLifecycle {
                state: self.state.clone(),
                finished: false,
            },
        })
    }

    pub(super) async fn write(&self, item: Item, name: &str) -> SilkwormResult<Item> {
        let sender = {
            let state = self.state.lock().map_err(|_| writer_lock_error())?;
            let WriterState::Open(sender) = &*state else {
                return Err(SilkwormError::Pipeline(format!("{name} not opened")));
            };
            sender.clone()
        };
        let (reply, result) = oneshot::channel();
        sender
            .send(Command::Write(item, reply))
            .await
            .map_err(|_| writer_stopped())?;
        result.await.map_err(|_| writer_stopped())?
    }

    pub(super) async fn close(&self) -> SilkwormResult<()> {
        let sender = {
            let mut state = self.state.lock().map_err(|_| writer_lock_error())?;
            match &*state {
                WriterState::Closed => return Ok(()),
                WriterState::Open(sender) => {
                    let sender = sender.clone();
                    *state = WriterState::Closing;
                    sender
                }
                WriterState::Opening | WriterState::Closing => {
                    return Err(SilkwormError::Pipeline(
                        "Pipeline is changing state".to_string(),
                    ));
                }
            }
        };
        let (reply, result) = oneshot::channel();
        sender
            .send(Command::Close(reply))
            .await
            .map_err(|_| writer_stopped())?;
        result.await.map_err(|_| writer_stopped())?
    }
}

fn writer_lock_error() -> SilkwormError {
    SilkwormError::Pipeline("Pipeline writer lock poisoned".to_string())
}

fn writer_stopped() -> SilkwormError {
    SilkwormError::Pipeline("Pipeline writer stopped".to_string())
}

async fn run_writer<W: AsyncWrite + Unpin>(
    mut writer: W,
    mut format: RecordFormat,
    footer: Vec<u8>,
    mut receiver: mpsc::Receiver<Command>,
    logger: Logger,
    mut lifecycle: WriterLifecycle,
) {
    let mut buffer = Vec::new();
    let mut failure = None;
    let mut close_reply = None;
    while let Some(command) = receiver.recv().await {
        match command {
            Command::Write(item, reply) => {
                let result = if let Some(message) = &failure {
                    Err(SilkwormError::Pipeline(format!(
                        "Pipeline writer failed: {message}"
                    )))
                } else {
                    async {
                        format.encode(&item, &mut buffer)?;
                        writer.write_all(&buffer).await?;
                        format.commit_record();
                        Ok(item)
                    }
                    .await
                };
                if failure.is_none()
                    && let Err(err) = &result
                {
                    failure = Some(err.to_string());
                    logger.error("Pipeline write failed", &[("error", err.to_string())]);
                }
                // A cancelled caller still gets its accepted record written.
                let _ = reply.send(result);
            }
            Command::Close(reply) => {
                receiver.close();
                close_reply = Some(reply);
                // Drain all commands already accepted before writing the footer.
            }
        }
    }
    let result = async {
        if let Some(message) = failure {
            return Err(SilkwormError::Pipeline(format!(
                "Pipeline writer failed: {message}"
            )));
        }
        writer.write_all(&footer).await?;
        writer.flush().await?;
        Ok(())
    }
    .await;
    if let Err(err) = &result {
        logger.error("Pipeline close failed", &[("error", err.to_string())]);
    }
    lifecycle.finish();
    if let Some(reply) = close_reply {
        let _ = reply.send(result);
    }
}

#[cfg(test)]
mod tests {
    use super::{FileWriter, RecordFormat};
    use crate::logging::get_logger;
    use crate::types::Item;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, duplex};

    fn start_writer(
        format: RecordFormat,
        footer: &[u8],
    ) -> (Arc<FileWriter>, tokio::io::DuplexStream) {
        let (sink, source) = duplex(1);
        let writer = Arc::new(FileWriter::default());
        writer
            .begin_open()
            .unwrap()
            .start_writer(
                sink,
                format,
                footer.to_vec(),
                get_logger("writer-test", None),
            )
            .unwrap();
        (writer, source)
    }

    #[tokio::test]
    async fn cancelled_write_completes_accepted_record_and_keeps_writer_usable() {
        let (writer, mut source) = start_writer(RecordFormat::JsonLines, &[]);
        let caller = {
            let writer = writer.clone();
            tokio::spawn(async move { writer.write(Item::from("first"), "test").await })
        };
        let mut first_byte = [0];
        // Reading the first byte proves the actor has accepted the command and
        // started a record too large for the one-byte transport buffer.
        source.read_exact(&mut first_byte).await.unwrap();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());

        let reader = tokio::spawn(async move {
            let mut output = first_byte.to_vec();
            source.read_to_end(&mut output).await.unwrap();
            output
        });
        writer.write(Item::from("second"), "test").await.unwrap();
        writer.close().await.unwrap();
        let output = tokio::time::timeout(Duration::from_secs(2), reader)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(output, b"\"first\"\n\"second\"\n");
    }

    #[tokio::test]
    async fn cancelled_close_still_finishes_footer_and_releases_lifecycle() {
        let (writer, mut source) = start_writer(RecordFormat::JsonLines, b"footer");
        let caller = {
            let writer = writer.clone();
            tokio::spawn(async move { writer.close().await })
        };
        let mut first_byte = [0];
        source.read_exact(&mut first_byte).await.unwrap();
        assert!(
            writer.begin_open().is_err(),
            "cannot reopen a writer still closing"
        );
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        let mut output = first_byte.to_vec();
        tokio::time::timeout(Duration::from_secs(2), source.read_to_end(&mut output))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(output, b"footer");
        let opening = writer.begin_open().expect("close must release lifecycle");
        drop(opening);
    }

    #[tokio::test]
    async fn dropped_pipeline_drains_commands_and_closes_writer() {
        use std::future::{Future, poll_fn};
        use std::task::Poll;
        let (writer, mut source) = start_writer(RecordFormat::JsonLines, b"footer");
        let mut write = Box::pin(writer.write(Item::from("accepted"), "test"));
        assert!(poll_fn(|cx| Poll::Ready(write.as_mut().poll(cx).is_pending())).await);
        drop(write);
        drop(writer);
        let mut output = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), source.read_to_end(&mut output))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(output, b"\"accepted\"\nfooter");
    }

    #[tokio::test]
    async fn writer_reports_io_failure_to_write_and_close() {
        let (writer, source) = start_writer(RecordFormat::JsonLines, &[]);
        drop(source);
        assert!(writer.write(Item::from("first"), "test").await.is_err());
        assert!(writer.write(Item::from("second"), "test").await.is_err());
        assert!(writer.close().await.is_err());
        assert!(writer.begin_open().is_ok());
    }

    #[test]
    fn cancelled_open_releases_lifecycle_reservation() {
        let writer = FileWriter::default();
        let opening = writer.begin_open().unwrap();
        assert!(writer.begin_open().is_err());
        drop(opening);
        assert!(writer.begin_open().is_ok());
    }
}
