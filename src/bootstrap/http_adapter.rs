use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::mpsc;
use std::thread;

use serde_json::{Value, json};

use crate::bootstrap::flow_runner::{PromptAdapter, Question};
use crate::error::{DeployerError, Result};

pub struct HttpPromptAdapter {
    listener: TcpListener,
    bind_addr: SocketAddr,
    timeout: std::time::Duration,
}

impl HttpPromptAdapter {
    pub fn bind(addr: &str, timeout: std::time::Duration) -> Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let bind_addr = listener.local_addr()?;
        Ok(Self {
            listener,
            bind_addr,
            timeout,
        })
    }

    pub fn bound_addr(&self) -> SocketAddr {
        self.bind_addr
    }
}

impl PromptAdapter for HttpPromptAdapter {
    fn ask(&mut self, questions: &[Question]) -> Result<Value> {
        let questions_json = json!({ "questions": questions });
        let (tx, rx) = mpsc::channel();

        let listener = self
            .listener
            .try_clone()
            .map_err(|err| DeployerError::Other(err.to_string()))?;
        let questions_clone: Vec<Question> = questions.to_vec();
        let timeout = self.timeout;
        let start = std::time::Instant::now();

        thread::spawn(move || {
            for stream in listener.incoming() {
                if start.elapsed() > timeout {
                    break;
                }
                match stream {
                    Ok(mut stream) => {
                        let _ = stream.set_read_timeout(Some(timeout));
                        // Read until headers and (optional) body are available or until timeout.
                        let mut buffer = Vec::new();
                        let mut header_end: Option<usize> = None;
                        let mut content_length: usize = 0;
                        loop {
                            let mut chunk = [0u8; 1024];
                            match stream.read(&mut chunk) {
                                Ok(0) => break,
                                Ok(n) => {
                                    buffer.extend_from_slice(&chunk[..n]);
                                    if header_end.is_none()
                                        && let Some(pos) =
                                            buffer.windows(4).position(|w| w == b"\r\n\r\n")
                                    {
                                        header_end = Some(pos + 4);
                                        let headers = String::from_utf8_lossy(&buffer[..pos + 4]);
                                        for line in headers.lines() {
                                            if let Some(value) =
                                                line.strip_prefix("Content-Length:")
                                            {
                                                content_length =
                                                    value.trim().parse::<usize>().unwrap_or(0);
                                            }
                                        }
                                    }
                                    if let Some(h_end) = header_end
                                        && buffer.len() >= h_end + content_length
                                    {
                                        break;
                                    }
                                    if n < chunk.len() {
                                        break;
                                    }
                                }
                                Err(ref e)
                                    if e.kind() == std::io::ErrorKind::WouldBlock
                                        || e.kind() == std::io::ErrorKind::TimedOut =>
                                {
                                    break;
                                }
                                Err(_) => break,
                            }
                            if start.elapsed() > timeout {
                                break;
                            }
                        }

                        let request = String::from_utf8_lossy(&buffer);
                        let mut lines = request.lines();
                        let request_line = lines.next().unwrap_or_default();
                        if request_line.starts_with("GET /schema") {
                            let body = serde_json::to_string(&questions_json).unwrap_or_default();
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                                body.len(),
                                body
                            );
                            let _ = stream.write_all(response.as_bytes());
                            let _ = stream.flush();
                        } else if request_line.starts_with("POST /answers") {
                            // read body
                            let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
                            if let Ok(value) = serde_json::from_str::<Value>(body) {
                                let _ = tx.send(value);
                                let response = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
                                let _ = stream.write_all(response.as_bytes());
                                let _ = stream.flush();
                                break;
                            } else {
                                let response =
                                    "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n";
                                let _ = stream.write_all(response.as_bytes());
                                let _ = stream.flush();
                            }
                        } else {
                            let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                            let _ = stream.write_all(response.as_bytes());
                            let _ = stream.flush();
                        }
                    }
                    Err(_) => continue,
                }
            }
        });

        let answers = rx
            .recv_timeout(self.timeout)
            .map_err(|err| DeployerError::Other(format!("no answers received: {err}")))?;

        // Ensure required answers exist
        let mut provided = serde_json::Map::new();
        for q in questions_clone {
            let value = answers
                .get(&q.id)
                .cloned()
                .or_else(|| q.default.as_ref().map(|d| Value::String(d.clone())))
                .ok_or_else(|| {
                    DeployerError::Config(format!("missing answer for question '{}'", q.id))
                })?;
            provided.insert(q.id, value);
        }

        Ok(Value::Object(provided))
    }
}
