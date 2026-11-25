use map_ai::{Curl, Transport};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

fn exchange(transport: &dyn Transport, status: u16) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/test", listener.local_addr().unwrap());
    let body = json!({"message":"地图 'A'\n\"B\" \\路", "tools":[]});
    let reply = json!({"answer":"含中文的响应", "status":status});
    std::thread::scope(|scope| {
        let server = scope.spawn(|| {
            let deadline = Instant::now() + Duration::from_secs(12);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("local HTTP accept: {e}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
            let mut reader = BufReader::new(&stream);
            let mut headers = String::new();
            loop {
                assert!(reader.read_line(&mut headers).unwrap() > 0);
                if headers.ends_with("\r\n\r\n") { break; }
            }
            let headers = headers.to_ascii_lowercase();
            assert!(headers.contains("x-atlas-test: local-only"));
            let length: usize = headers.lines().find_map(|s| s.strip_prefix("content-length:"))
                .unwrap().trim().parse().unwrap();
            if headers.contains("expect: 100-continue") {
                (&stream).write_all(b"HTTP/1.1 100 Continue\r\n\r\n").unwrap();
            }
            let mut received = vec![0; length];
            reader.read_exact(&mut received).unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&received).unwrap(), body);
            let text = reply.to_string();
            let location = if status == 302 { "Location: /follow\r\n" } else { "" };
            write!(stream, "HTTP/1.1 {status} Response\r\n{location}Content-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len()).unwrap();
        });
        let actual = transport.post(&url, &[("X-Atlas-Test".into(), "local-only".into())], &body, 8);
        server.join().unwrap();
        assert_eq!(actual.unwrap(), (status, reply.clone()));
    });
}

#[test]
fn curl_exchanges_unicode_json_and_http_errors() {
    exchange(&Curl, 200);
    exchange(&Curl, 302);
    exchange(&Curl, 429);
}

#[cfg(windows)]
#[test]
fn powershell_pipes_exchange_unicode_json_and_http_errors() {
    exchange(&map_ai::PowerShell, 200);
    exchange(&map_ai::PowerShell, 302);
    exchange(&map_ai::PowerShell, 429);
}
